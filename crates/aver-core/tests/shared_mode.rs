//! T102 — v0.9 starts by declaring a shared-mode feature flag.

#[test]
fn workspace_declares_shared_mode_feature() {
    let cargo_toml = include_str!("../../../Cargo.toml");

    assert!(cargo_toml.contains("shared_mode"));
}
