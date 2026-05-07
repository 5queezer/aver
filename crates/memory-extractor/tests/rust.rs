//! T55 — v0.3 starts with deterministic Tree-sitter Rust extraction.

use memory_extractor::extract_rust_functions;

#[test]
fn extract_rust_functions_finds_function_name() {
    let functions = extract_rust_functions("fn remember() { println!(\"hi\"); }").unwrap();

    assert_eq!(functions, vec!["remember".to_string()]);
}
