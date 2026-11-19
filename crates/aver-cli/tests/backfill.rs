use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    sync::mpsc,
    thread,
    time::Duration,
};

use aver_core::{ClaimWrite, Store, VECTOR_INDEX_DIM};

fn aver(memory_dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_aver"));
    command.arg("--memory-dir").arg(memory_dir).arg("backfill");
    for name in [
        "AVER_EMBEDDING_URL",
        "AVER_EMBEDDING_PROVIDER",
        "AVER_EMBEDDING_MODEL",
        "AVER_EMBEDDING_API_KEY_ENV",
        "OPENAI_API_KEY",
    ] {
        command.env_remove(name);
    }
    command
}

fn run(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "backfill failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn ollama_stub() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut head = String::new();
        let mut content_length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                content_length = value.trim().parse().unwrap();
            }
            if line == "\r\n" {
                break;
            }
            head.push_str(&line);
        }
        let mut request_body = vec![0; content_length];
        reader.read_exact(&mut request_body).unwrap();
        sender
            .send(head + &String::from_utf8(request_body).unwrap())
            .unwrap();

        let response_body = serde_json::json!({"embeddings": [vec![0.25; VECTOR_INDEX_DIM]]});
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response_body}",
            response_body.to_string().len()
        );
        reader.get_mut().write_all(response.as_bytes()).unwrap();
    });
    (url, receiver)
}

#[test]
fn backfill_command_fills_missing_vectors_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claim_id = store
        .add_claim(ClaimWrite::new("aver", "stores", "vectors", "test"))
        .unwrap();
    store
        .add_vector_chunk(claim_id, "aver stores vectors", "nomic-embed-text")
        .unwrap();
    store.close().unwrap();

    let (url, requests) = ollama_stub();
    let first = run(aver(dir.path()).env("AVER_EMBEDDING_URL", &url));
    assert_eq!(
        String::from_utf8(first.stdout).unwrap(),
        "backfill: embedded=1 indexed=1 total=1\n"
    );
    let request = requests.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(request.starts_with("POST /api/embed HTTP/1.1\r\n"));
    assert!(request.contains("\"input\":\"aver stores vectors\""));
    assert!(request.contains("\"truncate\":true"));

    let second = run(aver(dir.path()).env("AVER_EMBEDDING_URL", &url));
    assert_eq!(
        String::from_utf8(second.stdout).unwrap(),
        "backfill: embedded=0 indexed=1 total=1\n"
    );
}

#[test]
fn backfill_command_requires_an_embedding_provider() {
    let dir = tempfile::tempdir().unwrap();
    let output = aver(dir.path()).output().unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("no embedding provider configured; set AVER_EMBEDDING_URL")
    );
}
