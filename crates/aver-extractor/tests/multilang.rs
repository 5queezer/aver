use aver_extractor::{
    ExtractedFact, extract_go_facts, extract_go_functions, extract_python_facts,
    extract_python_functions, extract_typescript_classes, extract_typescript_facts,
    extract_typescript_functions,
};

#[test]
fn extract_python_functions_finds_function_name() {
    let functions = extract_python_functions("def remember():\n    return True\n").unwrap();

    assert_eq!(functions, vec!["remember".to_string()]);
}

#[test]
fn extract_python_facts_emits_file_defines_function_triple() {
    let facts = extract_python_facts("agent.py", "def recall():\n    pass\n").unwrap();

    assert_eq!(
        facts,
        vec![ExtractedFact {
            subject: "agent.py".to_string(),
            predicate: "defines".to_string(),
            object: "Function:recall".to_string(),
        }]
    );
}

#[test]
fn extract_typescript_functions_and_classes_find_symbol_names() {
    let source = "class Store extends BaseStore {}\nfunction remember() { return true; }";

    assert_eq!(
        extract_typescript_classes(source).unwrap(),
        vec!["Store".to_string()]
    );
    assert_eq!(
        extract_typescript_functions(source).unwrap(),
        vec!["remember".to_string()]
    );
}

#[test]
fn extract_typescript_facts_emits_class_extends_triple() {
    let facts = extract_typescript_facts("store.ts", "class Store extends BaseStore {}").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Class:BaseStore".to_string(),
    }));
}

#[test]
fn extract_go_functions_and_facts_emit_definitions() {
    let source = "package memory\nfunc Remember() bool { return true }\n";

    assert_eq!(
        extract_go_functions(source).unwrap(),
        vec!["Remember".to_string()]
    );
    assert_eq!(
        extract_go_facts("memory.go", source).unwrap(),
        vec![ExtractedFact {
            subject: "memory.go".to_string(),
            predicate: "defines".to_string(),
            object: "Function:Remember".to_string(),
        }]
    );
}
