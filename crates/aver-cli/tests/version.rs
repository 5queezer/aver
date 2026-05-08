//! `aver --version` supports installer verification.

use std::process::Command;

#[test]
fn aver_version_prints_package_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_aver"))
        .arg("--version")
        .output()
        .expect("aver binary should run");

    assert!(
        output.status.success(),
        "aver --version should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("aver {}\n", env!("CARGO_PKG_VERSION"))
    );
}
