#[test]
fn aver_server_binary_is_declared() {
    let manifest =
        std::fs::read_to_string(env!("CARGO_MANIFEST_DIR").to_string() + "/Cargo.toml").unwrap();
    assert!(manifest.contains("[[bin]]"));
    assert!(manifest.contains("aver-server"));
}
