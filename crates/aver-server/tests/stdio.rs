//! `aver-server --stdio`: the real binary driven over pipes.
//!
//! Every byte the child writes to stdout must be a JSON-RPC frame; anything
//! else would corrupt the parent's MCP session.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};

use serde_json::{Value, json};

struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

fn command(memory_dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_aver-server"));
    command
        .arg("--stdio")
        .env("AVER_MEMORY_DIR", memory_dir)
        .env_remove("AVER_DEFAULT_SCOPE")
        .env_remove("AVER_EMBEDDING_URL")
        .env_remove("AVER_RECALL_RECENCY_WEIGHT")
        .env_remove("AVER_DECISION_MODEL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

impl Session {
    fn start(mut command: Command) -> Self {
        let mut child = command.spawn().unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut session = Self {
            child,
            stdin,
            stdout,
            next_id: 0,
        };
        let init = session.request(
            "initialize",
            json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "stdio-test", "version": "0"},
            }),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "aver", "{init}");
        session.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        session
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Sends a request and returns its response. Every stdout line read on
    /// the way must be a JSON-RPC 2.0 frame.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let mut line = String::new();
            assert_ne!(
                self.stdout.read_line(&mut line).unwrap(),
                0,
                "stdout closed"
            );
            let frame = protocol_frame(&line);
            if frame["id"] == id {
                return frame;
            }
        }
    }

    /// Calls a tool and returns its JSON payload, failing on a tool error.
    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        let response = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        let result = &response["result"];
        assert!(response.get("error").is_none(), "{tool}: {response}");
        assert_ne!(result["isError"], true, "{tool}: {response}");
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{response}"));
        serde_json::from_str(text).unwrap_or_else(|_| panic!("{text}"))
    }

    /// Closes stdin and collects the rest of both streams and the status.
    fn finish(self) -> Output {
        let Self {
            child,
            stdin,
            mut stdout,
            ..
        } = self;
        drop(stdin);
        let mut rest = String::new();
        stdout.read_to_string(&mut rest).unwrap();
        for line in rest.lines() {
            protocol_frame(line);
        }
        child.wait_with_output().unwrap()
    }
}

fn protocol_frame(line: &str) -> Value {
    let frame: Value = serde_json::from_str(line.trim_end())
        .unwrap_or_else(|_| panic!("non-protocol bytes on stdout: {line:?}"));
    assert_eq!(
        frame["jsonrpc"], "2.0",
        "non-protocol frame on stdout: {line:?}"
    );
    frame
}

fn remember(session: &mut Session, subject: &str, object: &str) -> i64 {
    session.call(
        "remember_claim",
        json!({"subject": subject, "predicate": "uses", "object": object}),
    )["id"]
        .as_i64()
        .unwrap()
}

/// One session that exercises every stderr diagnostic reachable in stdio
/// mode (invalid recency setting at startup, failed embedding on a chunk
/// write and on recall) plus read, write and maintenance scopes, and still
/// produces nothing but protocol frames on stdout.
#[test]
fn stdio_session_grants_every_scope_and_keeps_stdout_for_the_protocol() {
    let dir = tempfile::tempdir().unwrap();
    let mut command = command(dir.path());
    command
        .env("AVER_DEFAULT_SCOPE", "proj/stdio-test")
        .env("AVER_RECALL_RECENCY_WEIGHT", "not-a-number")
        // Nothing listens on the discard port: every embedding call fails.
        .env("AVER_EMBEDDING_URL", "http://127.0.0.1:9")
        .env("AVER_EMBEDDING_TIMEOUT_SECS", "1");
    let mut session = Session::start(command);

    let tools = session.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"recall") && names.contains(&"consolidate"),
        "{names:?}"
    );

    // claims:write, with the scope taken from AVER_DEFAULT_SCOPE.
    let claim_id = remember(&mut session, "api-gateway", "envoy");
    // Chunk write with a failing embedder: stored text-only, reported on stderr.
    session.call(
        "add_vector_chunk",
        json!({"claim_id": claim_id, "text": "api-gateway uses envoy"}),
    );
    // claims:read. No embedded chunk exists, so recall answers by keyword.
    let recalled = session.call("recall", json!({"query": "api-gateway"}));
    assert_eq!(recalled["retrieval"], "keyword", "{recalled}");
    assert_eq!(recalled["triples"][0]["id"], claim_id);
    assert_eq!(recalled["triples"][0]["scope"], "proj/stdio-test");
    // Maintenance scope.
    session.call("consolidate", json!({}));

    let output = session.finish();
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("AVER_RECALL_RECENCY_WEIGHT"), "{stderr}");
    assert!(stderr.contains("chunk embedding failed"), "{stderr}");
}

#[test]
fn stdio_exits_non_zero_with_a_stderr_message_and_no_stdout_on_bad_setup() {
    let dir = tempfile::tempdir().unwrap();
    let not_a_dir = dir.path().join("file");
    std::fs::write(&not_a_dir, "x").unwrap();
    let mut bad_scope = command(dir.path());
    bad_scope.env("AVER_DEFAULT_SCOPE", "Not A Scope!");

    for mut command in [command(&not_a_dir), bad_scope] {
        let output = command.stdin(Stdio::null()).output().unwrap();
        assert!(!output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(!output.stderr.is_empty(), "{output:?}");
    }
}

/// Two stdio children on one store, as when a parent starts the server
/// twice: interleaved writes from both, each sees the other's claims, no
/// log line is lost, and replaying the log reproduces the live projection.
#[test]
fn two_stdio_children_share_one_store_and_the_log_replays_to_the_same_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = Session::start(command(dir.path()));
    let mut second = Session::start(command(dir.path()));

    const WRITES: usize = 20;
    let (first_ids, second_ids) = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            (0..WRITES)
                .map(|n| remember(&mut first, "service-a", &format!("dep-{n}")))
                .collect::<Vec<_>>()
        });
        let b = scope.spawn(|| {
            (0..WRITES)
                .map(|n| remember(&mut second, "service-b", &format!("dep-{n}")))
                .collect::<Vec<_>>()
        });
        (a.join().unwrap(), b.join().unwrap())
    });
    let mut ids: Vec<i64> = first_ids.iter().chain(&second_ids).copied().collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 2 * WRITES, "colliding claim ids");

    // Each child reads what the other wrote.
    let seen = second.call("recall", json!({"query": "service-a", "top_k": 50}));
    assert_eq!(seen["triples"].as_array().unwrap().len(), WRITES, "{seen}");
    assert!(first.finish().status.success());
    assert!(second.finish().status.success());

    let log = std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap();
    let logged = log
        .lines()
        .filter(|line| protocol_free_kind(line) == "add_claim")
        .count();
    assert_eq!(logged, 2 * WRITES);

    let snapshot = |dir: &Path| -> Vec<String> {
        let store = aver_core::Store::open(dir).unwrap();
        ids.iter()
            .map(|id| format!("{:?}", store.get_claim(*id).unwrap()))
            .collect()
    };
    let live = snapshot(dir.path());
    aver_core::replay(dir.path(), true).unwrap();
    assert_eq!(live, snapshot(dir.path()));
}

fn protocol_free_kind(line: &str) -> String {
    serde_json::from_str::<Value>(line).unwrap()["kind"]
        .as_str()
        .unwrap()
        .to_string()
}

/// An embedding endpoint that accepts the connection and never answers must
/// cost one timeout, not hang the session.
#[test]
fn stdio_embedding_call_is_bounded_by_the_configured_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut command = command(dir.path());
    command
        .env(
            "AVER_EMBEDDING_URL",
            format!("http://{}", silent.local_addr().unwrap()),
        )
        .env("AVER_EMBEDDING_TIMEOUT_SECS", "1");
    let mut session = Session::start(command);
    let claim_id = remember(&mut session, "api-gateway", "envoy");

    let started = std::time::Instant::now();
    session.call(
        "add_vector_chunk",
        json!({"claim_id": claim_id, "text": "api-gateway uses envoy"}),
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed >= std::time::Duration::from_millis(900),
        "{elapsed:?}"
    );
    assert!(elapsed < std::time::Duration::from_secs(5), "{elapsed:?}");
    assert!(session.finish().status.success());
}
