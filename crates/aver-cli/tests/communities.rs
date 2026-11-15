use std::process::Command;

#[test]
fn communities_prints_weighted_members_score_and_bridge_nodes() {
    let dir = std::env::temp_dir().join(format!("aver-cli-communities-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    for args in [
        [
            "add-triple",
            "A",
            "depends_on",
            "B",
            "--source",
            "strong",
            "--confidence",
            "0.95",
        ],
        [
            "add-triple",
            "B",
            "depends_on",
            "C",
            "--source",
            "strong",
            "--confidence",
            "0.95",
        ],
        [
            "add-triple",
            "C",
            "depends_on",
            "D",
            "--source",
            "weak",
            "--confidence",
            "0.1",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_aver"))
            .arg("--memory-dir")
            .arg(dir.to_str().unwrap())
            .args(args)
            .output()
            .expect("aver add-triple should run");
        assert!(output.status.success(), "setup failed: {output:?}");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_aver"))
        .args(["--memory-dir", dir.to_str().unwrap(), "communities"])
        .output()
        .expect("aver communities should run");

    assert!(
        output.status.success(),
        "communities command should succeed"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("community:A-B-C score="), "got {stdout:?}");
    assert!(stdout.contains("members=A,B,C"), "got {stdout:?}");
    assert!(stdout.contains("bridges=C"), "got {stdout:?}");
    assert!(stdout.contains("community:D score=0.000"), "got {stdout:?}");

    let _ = std::fs::remove_dir_all(&dir);
}
