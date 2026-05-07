//! T55 — v0.3 starts with deterministic Tree-sitter Rust extraction.

use aver_extractor::{
    ExtractedFact, extract_rust_calls, extract_rust_enums, extract_rust_facts,
    extract_rust_functions, extract_rust_imports, extract_rust_structs, extract_rust_tests,
    extract_rust_traits, map_rust_tests_to_functions,
};

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

#[test]
fn extract_rust_structs_finds_struct_name() {
    let structs = extract_rust_structs("struct Claim { text: String }").unwrap();

    assert_eq!(structs, vec!["Claim".to_string()]);
}

#[test]
fn extract_rust_enums_finds_enum_name() {
    let enums = extract_rust_enums("enum MemoryError { ParseFailed }").unwrap();

    assert_eq!(enums, vec!["MemoryError".to_string()]);
}

#[test]
fn extract_rust_traits_finds_trait_name() {
    let traits = extract_rust_traits("trait EmbeddingClient { fn embed(&self); }").unwrap();

    assert_eq!(traits, vec!["EmbeddingClient".to_string()]);
}

#[test]
fn extract_rust_tests_finds_test_function_name() {
    let tests = extract_rust_tests("#[test]\nfn add_claim_persists_log_first() {}").unwrap();

    assert_eq!(tests, vec!["add_claim_persists_log_first".to_string()]);
}

#[test]
fn map_rust_tests_to_functions_uses_test_name_prefix() {
    let mappings = map_rust_tests_to_functions(
        "fn add_claim() {}\n#[test]\nfn add_claim_persists_log_first() {}",
    )
    .unwrap();

    assert_eq!(
        mappings,
        vec![(
            "add_claim_persists_log_first".to_string(),
            "add_claim".to_string()
        )]
    );
}

#[test]
fn extract_rust_facts_emits_file_defines_function_triple() {
    let facts = extract_rust_facts("src/lib.rs", "fn remember() {}").unwrap();

    assert_eq!(
        facts,
        vec![ExtractedFact {
            subject: "src/lib.rs".to_string(),
            predicate: "defines".to_string(),
            object: "Function:remember".to_string(),
        }]
    );
}

#[test]
fn extract_rust_facts_emits_file_imports_module_triple() {
    let facts = extract_rust_facts("src/lib.rs", "use std::fs;").unwrap();

    assert_eq!(
        facts,
        vec![ExtractedFact {
            subject: "src/lib.rs".to_string(),
            predicate: "imports".to_string(),
            object: "Module:std::fs".to_string(),
        }]
    );
}

#[test]
fn extract_rust_facts_emits_function_calls_function_triple() {
    let facts = extract_rust_facts("src/lib.rs", "fn remember() { recall(); }").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Function:remember".to_string(),
        predicate: "calls".to_string(),
        object: "Function:recall".to_string(),
    }));
}

#[test]
fn extract_rust_facts_emits_test_covers_function_triple() {
    let facts = extract_rust_facts(
        "src/lib.rs",
        "fn add_claim() {}\n#[test]\nfn add_claim_persists_log_first() {}",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Function:add_claim_persists_log_first".to_string(),
        predicate: "tests".to_string(),
        object: "Function:add_claim".to_string(),
    }));
}

#[test]
fn extract_rust_facts_emits_file_defines_struct_triple() {
    let facts = extract_rust_facts("src/lib.rs", "struct Claim { text: String }").unwrap();

    assert_eq!(
        facts,
        vec![ExtractedFact {
            subject: "src/lib.rs".to_string(),
            predicate: "defines".to_string(),
            object: "Struct:Claim".to_string(),
        }]
    );
}

#[test]
fn extract_rust_facts_emits_file_defines_enum_triple() {
    let facts = extract_rust_facts("src/lib.rs", "enum MemoryError { ParseFailed }").unwrap();

    assert_eq!(
        facts,
        vec![ExtractedFact {
            subject: "src/lib.rs".to_string(),
            predicate: "defines".to_string(),
            object: "Enum:MemoryError".to_string(),
        }]
    );
}

#[test]
fn extract_rust_facts_emits_impl_defines_method_triple() {
    let facts = extract_rust_facts("src/lib.rs", "impl Store { fn add_claim(&self) {} }").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Type:Store".to_string(),
        predicate: "defines".to_string(),
        object: "Function:Store::add_claim".to_string(),
    }));
}

#[test]
fn extract_rust_facts_emits_file_defines_trait_triple() {
    let facts =
        extract_rust_facts("src/lib.rs", "trait EmbeddingClient { fn embed(&self); }").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "src/lib.rs".to_string(),
        predicate: "defines".to_string(),
        object: "Trait:EmbeddingClient".to_string(),
    }));
}

#[test]
fn extract_rust_facts_emits_qualified_impl_method_call_triple() {
    let facts = extract_rust_facts(
        "src/lib.rs",
        "impl Store { fn add_claim(&self) { self.append_log(); } }",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Function:Store::add_claim".to_string(),
        predicate: "calls".to_string(),
        object: "Function:self.append_log".to_string(),
    }));
}

#[test]
fn extract_rust_facts_emits_file_defines_module_triple() {
    let facts = extract_rust_facts("src/lib.rs", "mod embedding { fn embed() {} }").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "src/lib.rs".to_string(),
        predicate: "defines".to_string(),
        object: "Module:embedding".to_string(),
    }));
}

#[test]
fn extract_rust_facts_emits_module_defines_function_triple() {
    let facts = extract_rust_facts("src/lib.rs", "mod embedding { fn embed() {} }").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Module:embedding".to_string(),
        predicate: "defines".to_string(),
        object: "Function:embedding::embed".to_string(),
    }));
}

#[test]
fn extract_rust_facts_emits_module_qualified_function_call_triple() {
    let facts = extract_rust_facts(
        "src/lib.rs",
        "mod embedding { fn embed() { normalize(); } fn normalize() {} }",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Function:embedding::embed".to_string(),
        predicate: "calls".to_string(),
        object: "Function:embedding::normalize".to_string(),
    }));
}

#[test]
fn extract_rust_facts_emits_nested_module_definition_triple() {
    let facts =
        extract_rust_facts("src/lib.rs", "mod outer { mod inner { fn run() {} } }").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Module:outer".to_string(),
        predicate: "defines".to_string(),
        object: "Module:outer::inner".to_string(),
    }));
}

#[test]
fn extract_rust_facts_emits_module_qualified_impl_method_triple() {
    let facts = extract_rust_facts(
        "src/lib.rs",
        "mod storage { impl Store { fn add_claim(&self) {} } }",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Type:storage::Store".to_string(),
        predicate: "defines".to_string(),
        object: "Function:storage::Store::add_claim".to_string(),
    }));
}

#[test]
fn extract_rust_facts_emits_module_qualified_impl_method_call_triple() {
    let facts = extract_rust_facts(
        "src/lib.rs",
        "mod storage { impl Store { fn add_claim(&self) { self.append_log(); } } }",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Function:storage::Store::add_claim".to_string(),
        predicate: "calls".to_string(),
        object: "Function:self.append_log".to_string(),
    }));
}

#[test]
fn extract_rust_facts_emits_module_qualified_impl_trait_triple() {
    let facts = extract_rust_facts(
        "src/lib.rs",
        "mod embedding { impl EmbeddingClient for OllamaClient { fn embed(&self) {} } }",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Type:embedding::OllamaClient".to_string(),
        predicate: "implements".to_string(),
        object: "Trait:embedding::EmbeddingClient".to_string(),
    }));
}

#[test]
fn extract_rust_facts_emits_module_defines_trait_triple() {
    let facts = extract_rust_facts(
        "src/lib.rs",
        "mod embedding { trait EmbeddingClient { fn embed(&self); } }",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Module:embedding".to_string(),
        predicate: "defines".to_string(),
        object: "Trait:embedding::EmbeddingClient".to_string(),
    }));
}

#[test]
fn extract_rust_facts_emits_module_defines_struct_triple() {
    let facts =
        extract_rust_facts("src/lib.rs", "mod storage { struct Store { id: u64 } }").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Module:storage".to_string(),
        predicate: "defines".to_string(),
        object: "Struct:storage::Store".to_string(),
    }));
}

#[test]
fn extract_rust_facts_emits_module_defines_enum_triple() {
    let facts = extract_rust_facts(
        "src/lib.rs",
        "mod errors { enum MemoryError { ParseFailed } }",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Module:errors".to_string(),
        predicate: "defines".to_string(),
        object: "Enum:errors::MemoryError".to_string(),
    }));
}

#[test]
fn extract_rust_facts_emits_type_implements_trait_triple() {
    let facts = extract_rust_facts(
        "src/lib.rs",
        "impl EmbeddingClient for OllamaEmbeddingClient { fn embed(&self) {} }",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Type:OllamaEmbeddingClient".to_string(),
        predicate: "implements".to_string(),
        object: "Trait:EmbeddingClient".to_string(),
    }));
}
