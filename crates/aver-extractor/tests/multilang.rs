use aver_extractor::{
    ExtractedFact, extract_c_facts, extract_c_functions, extract_cpp_classes, extract_cpp_facts,
    extract_cpp_functions, extract_csharp_classes, extract_csharp_facts, extract_csharp_functions,
    extract_go_facts, extract_go_functions, extract_java_classes, extract_java_facts,
    extract_java_functions, extract_java_interfaces, extract_javascript_classes,
    extract_javascript_facts, extract_javascript_functions, extract_kotlin_classes,
    extract_kotlin_facts, extract_kotlin_functions, extract_php_classes, extract_php_facts,
    extract_php_functions, extract_python_facts, extract_python_functions, extract_ruby_classes,
    extract_ruby_facts, extract_ruby_functions, extract_swift_classes, extract_swift_facts,
    extract_swift_functions, extract_typescript_classes, extract_typescript_facts,
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

#[test]
fn extract_common_language_basic_symbols_emit_definition_facts() {
    assert_eq!(
        extract_javascript_functions("class Store {}\nfunction remember() { return true; }")
            .unwrap(),
        vec!["remember".to_string()]
    );
    assert_eq!(
        extract_javascript_classes("class Store {}\nfunction remember() { return true; }").unwrap(),
        vec!["Store".to_string()]
    );
    assert!(
        extract_javascript_facts("store.js", "class Store {}")
            .unwrap()
            .contains(&ExtractedFact {
                subject: "store.js".to_string(),
                predicate: "defines".to_string(),
                object: "Class:Store".to_string(),
            })
    );

    let java = "class Memory { void remember() {} }\ninterface Recallable {}";
    assert_eq!(
        extract_java_functions(java).unwrap(),
        vec!["remember".to_string()]
    );
    assert_eq!(
        extract_java_classes(java).unwrap(),
        vec!["Memory".to_string()]
    );
    assert_eq!(
        extract_java_interfaces(java).unwrap(),
        vec!["Recallable".to_string()]
    );
    assert!(
        extract_java_facts("Memory.java", java)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "Memory.java".to_string(),
                predicate: "defines".to_string(),
                object: "Interface:Recallable".to_string(),
            })
    );

    let c = "int remember(void) { return 1; }";
    assert_eq!(
        extract_c_functions(c).unwrap(),
        vec!["remember".to_string()]
    );
    assert_eq!(
        extract_c_facts("memory.c", c).unwrap(),
        vec![ExtractedFact {
            subject: "memory.c".to_string(),
            predicate: "defines".to_string(),
            object: "Function:remember".to_string(),
        }]
    );

    let cpp = "class Store {};\nint remember() { return 1; }";
    assert_eq!(
        extract_cpp_functions(cpp).unwrap(),
        vec!["remember".to_string()]
    );
    assert_eq!(extract_cpp_classes(cpp).unwrap(), vec!["Store".to_string()]);
    assert!(
        extract_cpp_facts("store.cpp", cpp)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "store.cpp".to_string(),
                predicate: "defines".to_string(),
                object: "Class:Store".to_string(),
            })
    );

    let csharp = "class Store { void Remember() {} }";
    assert_eq!(
        extract_csharp_functions(csharp).unwrap(),
        vec!["Remember".to_string()]
    );
    assert_eq!(
        extract_csharp_classes(csharp).unwrap(),
        vec!["Store".to_string()]
    );
    assert!(
        extract_csharp_facts("Store.cs", csharp)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "Store.cs".to_string(),
                predicate: "defines".to_string(),
                object: "Class:Store".to_string(),
            })
    );

    let ruby = "class Store\n  def remember\n    true\n  end\nend";
    assert_eq!(
        extract_ruby_functions(ruby).unwrap(),
        vec!["remember".to_string()]
    );
    assert_eq!(
        extract_ruby_classes(ruby).unwrap(),
        vec!["Store".to_string()]
    );
    assert!(
        extract_ruby_facts("store.rb", ruby)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "store.rb".to_string(),
                predicate: "defines".to_string(),
                object: "Class:Store".to_string(),
            })
    );

    let php = "<?php class Store { function remember() { return true; } }";
    assert_eq!(
        extract_php_functions(php).unwrap(),
        vec!["remember".to_string()]
    );
    assert_eq!(extract_php_classes(php).unwrap(), vec!["Store".to_string()]);
    assert!(
        extract_php_facts("store.php", php)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "store.php".to_string(),
                predicate: "defines".to_string(),
                object: "Class:Store".to_string(),
            })
    );

    let kotlin = "class Store { fun remember(): Boolean = true }";
    assert_eq!(
        extract_kotlin_functions(kotlin).unwrap(),
        vec!["remember".to_string()]
    );
    assert_eq!(
        extract_kotlin_classes(kotlin).unwrap(),
        vec!["Store".to_string()]
    );
    assert!(
        extract_kotlin_facts("Store.kt", kotlin)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "Store.kt".to_string(),
                predicate: "defines".to_string(),
                object: "Class:Store".to_string(),
            })
    );

    let swift = "class Store { func remember() -> Bool { true } }";
    assert_eq!(
        extract_swift_functions(swift).unwrap(),
        vec!["remember".to_string()]
    );
    assert_eq!(
        extract_swift_classes(swift).unwrap(),
        vec!["Store".to_string()]
    );
    assert!(
        extract_swift_facts("Store.swift", swift)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "Store.swift".to_string(),
                predicate: "defines".to_string(),
                object: "Class:Store".to_string(),
            })
    );
}
