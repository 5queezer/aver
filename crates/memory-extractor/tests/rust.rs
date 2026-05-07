//! T55 — v0.3 starts with deterministic Tree-sitter Rust extraction.

use memory_extractor::{extract_rust_calls, extract_rust_functions, extract_rust_imports};

#[test]
fn extract_rust_functions_finds_function_name() {
    let functions = extract_rust_functions("fn remember() { println!(\"hi\"); }").unwrap();

    assert_eq!(functions, vec!["remember".to_string()]);
}

#[test]
fn extract_rust_imports_finds_use_path() {
    let imports = extract_rust_imports("use std::fs;\nfn main() {}").unwrap();

    assert_eq!(imports, vec!["std::fs".to_string()]);
}

#[test]
fn extract_rust_calls_finds_called_function_name() {
    let calls = extract_rust_calls("fn main() { remember(); }").unwrap();

    assert_eq!(calls, vec!["remember".to_string()]);
}
