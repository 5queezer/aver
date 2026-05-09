use aver_extractor::{
    ExtractedFact, extract_c_enums, extract_c_facts, extract_c_functions, extract_c_structs,
    extract_c_type_aliases, extract_cpp_classes, extract_cpp_enums, extract_cpp_facts,
    extract_cpp_functions, extract_cpp_namespaces, extract_cpp_structs, extract_cpp_type_aliases,
    extract_csharp_classes, extract_csharp_delegates, extract_csharp_enums, extract_csharp_facts,
    extract_csharp_functions, extract_csharp_interfaces, extract_csharp_namespaces,
    extract_csharp_records, extract_csharp_structs, extract_facts_for_path, extract_go_facts,
    extract_go_functions, extract_go_interfaces, extract_go_structs, extract_java_annotations,
    extract_java_classes, extract_java_enums, extract_java_facts, extract_java_functions,
    extract_java_interfaces, extract_java_packages, extract_java_records,
    extract_javascript_classes, extract_javascript_facts, extract_javascript_functions,
    extract_kotlin_classes, extract_kotlin_enums, extract_kotlin_facts, extract_kotlin_functions,
    extract_kotlin_interfaces, extract_php_classes, extract_php_enums, extract_php_facts,
    extract_php_functions, extract_php_interfaces, extract_php_namespaces, extract_php_traits,
    extract_python_classes, extract_python_facts, extract_python_functions, extract_ruby_classes,
    extract_ruby_facts, extract_ruby_functions, extract_ruby_modules, extract_swift_actors,
    extract_swift_classes, extract_swift_enums, extract_swift_facts, extract_swift_functions,
    extract_swift_protocols, extract_swift_structs, extract_typescript_classes,
    extract_typescript_enums, extract_typescript_facts, extract_typescript_functions,
    extract_typescript_interfaces, extract_typescript_type_aliases,
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
fn extract_python_facts_emit_class_extends_triple() {
    let facts = extract_python_facts("store.py", "class Store(BaseStore):\n    pass\n").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Class:BaseStore".to_string(),
    }));
}

#[test]
fn extract_python_facts_emit_multiple_class_extends_triples() {
    let facts =
        extract_python_facts("store.py", "class Store(BaseStore, Auditable):\n    pass\n").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Class:BaseStore".to_string(),
    }));
    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Class:Auditable".to_string(),
    }));
}

#[test]
fn extract_python_classes_and_go_type_symbols_emit_definition_facts() {
    let python = "class Store:\n    pass\n";
    assert_eq!(
        extract_python_classes(python).unwrap(),
        vec!["Store".to_string()]
    );
    assert!(
        extract_python_facts("store.py", python)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "store.py".to_string(),
                predicate: "defines".to_string(),
                object: "Class:Store".to_string(),
            })
    );

    let go = "package memory\ntype Chunk struct {}\ntype Recallable interface {}\n";
    assert_eq!(extract_go_structs(go).unwrap(), vec!["Chunk".to_string()]);
    assert_eq!(
        extract_go_interfaces(go).unwrap(),
        vec!["Recallable".to_string()]
    );
    assert!(
        extract_go_facts("memory.go", go)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "memory.go".to_string(),
                predicate: "defines".to_string(),
                object: "Interface:Recallable".to_string(),
            })
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
fn extract_javascript_and_typescript_arrow_function_variables() {
    assert_eq!(
        extract_javascript_functions("const remember = () => true;").unwrap(),
        vec!["remember".to_string()]
    );
    assert_eq!(
        extract_typescript_functions("const recall = (): boolean => true;").unwrap(),
        vec!["recall".to_string()]
    );
}

#[test]
fn extract_typescript_facts_emit_interface_extends_triple() {
    let facts = extract_typescript_facts(
        "memory.ts",
        "interface Recallable extends BaseRecallable {}",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Interface:Recallable".to_string(),
        predicate: "extends".to_string(),
        object: "Interface:BaseRecallable".to_string(),
    }));
}

#[test]
fn extract_typescript_type_symbols_emit_definition_facts() {
    let source = "interface Recallable {} type MemoryId = string; enum MemoryKind { Episodic }";

    assert_eq!(
        extract_typescript_interfaces(source).unwrap(),
        vec!["Recallable".to_string()]
    );
    assert_eq!(
        extract_typescript_type_aliases(source).unwrap(),
        vec!["MemoryId".to_string()]
    );
    assert_eq!(
        extract_typescript_enums(source).unwrap(),
        vec!["MemoryKind".to_string()]
    );
    let facts = extract_typescript_facts("memory.ts", source).unwrap();
    assert!(facts.contains(&ExtractedFact {
        subject: "memory.ts".to_string(),
        predicate: "defines".to_string(),
        object: "Interface:Recallable".to_string(),
    }));
    assert!(facts.contains(&ExtractedFact {
        subject: "memory.ts".to_string(),
        predicate: "defines".to_string(),
        object: "TypeAlias:MemoryId".to_string(),
    }));
    assert!(facts.contains(&ExtractedFact {
        subject: "memory.ts".to_string(),
        predicate: "defines".to_string(),
        object: "Enum:MemoryKind".to_string(),
    }));
}

#[test]
fn extract_javascript_facts_emit_class_extends_triple() {
    let facts = extract_javascript_facts("store.js", "class Store extends BaseStore {}").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Class:BaseStore".to_string(),
    }));
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
fn extract_typescript_facts_emit_class_implements_triple() {
    let facts =
        extract_typescript_facts("store.ts", "class Store implements Recallable {}").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "implements".to_string(),
        object: "Interface:Recallable".to_string(),
    }));
}

#[test]
fn extract_go_facts_emit_struct_embedding_extends_triple() {
    let facts = extract_go_facts(
        "store.go",
        "package memory\ntype BaseStore struct {}\ntype Store struct { BaseStore }",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Struct:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Struct:BaseStore".to_string(),
    }));
}

#[test]
fn extract_go_facts_do_not_treat_named_struct_fields_as_extends() {
    let facts = extract_go_facts(
        "store.go",
        "package memory\ntype BaseStore struct {}\ntype Store struct { base BaseStore }",
    )
    .unwrap();

    assert!(!facts.contains(&ExtractedFact {
        subject: "Struct:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Struct:BaseStore".to_string(),
    }));
}

#[test]
fn extract_go_facts_emit_interface_extends_triple() {
    let facts = extract_go_facts(
        "memory.go",
        "package memory\ntype Recallable interface { BaseRecallable }",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Interface:Recallable".to_string(),
        predicate: "extends".to_string(),
        object: "Interface:BaseRecallable".to_string(),
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

#[test]
fn extract_swift_facts_emit_class_extends_triple() {
    let facts = extract_swift_facts("Store.swift", "class Store: BaseStore {}").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Class:BaseStore".to_string(),
    }));
}

#[test]
fn extract_csharp_facts_emit_class_extends_triple() {
    let facts = extract_csharp_facts("Store.cs", "class Store : BaseStore {}").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Class:BaseStore".to_string(),
    }));
}

#[test]
fn extract_cpp_facts_emit_struct_extends_triple() {
    let facts = extract_cpp_facts("store.hpp", "struct Store : BaseStore {};").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Struct:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Struct:BaseStore".to_string(),
    }));
}

#[test]
fn extract_cpp_facts_emit_class_extends_triple() {
    let facts = extract_cpp_facts("store.hpp", "class Store : public BaseStore {};").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Class:BaseStore".to_string(),
    }));
}

#[test]
fn extract_cpp_facts_emit_multiple_class_extends_triples() {
    let facts = extract_cpp_facts(
        "store.hpp",
        "class Store : public BaseStore, public Auditable {};",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Class:BaseStore".to_string(),
    }));
    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Class:Auditable".to_string(),
    }));
}

#[test]
fn extract_java_facts_emit_interface_extends_triple() {
    let facts = extract_java_facts(
        "Recallable.java",
        "interface Recallable extends BaseRecallable {}",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Interface:Recallable".to_string(),
        predicate: "extends".to_string(),
        object: "Interface:BaseRecallable".to_string(),
    }));
}

#[test]
fn extract_java_facts_emit_class_implements_triple() {
    let facts = extract_java_facts("Store.java", "class Store implements Recallable {}").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "implements".to_string(),
        object: "Interface:Recallable".to_string(),
    }));
}

#[test]
fn extract_java_facts_emit_class_extends_triple() {
    let facts = extract_java_facts("Store.java", "class Store extends BaseStore {}").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Class:BaseStore".to_string(),
    }));
}

#[test]
fn extract_java_packages_emit_definition_facts() {
    let source = "package memory.core; public class Store {}";

    assert_eq!(
        extract_java_packages(source).unwrap(),
        vec!["memory.core".to_string()]
    );
    assert!(
        extract_java_facts("Store.java", source)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "Store.java".to_string(),
                predicate: "defines".to_string(),
                object: "Package:memory.core".to_string(),
            })
    );
}

#[test]
fn extract_php_facts_emit_class_extends_triple() {
    let facts = extract_php_facts("Store.php", "<?php class Store extends BaseStore {}").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Class:BaseStore".to_string(),
    }));
}

#[test]
fn extract_php_facts_emit_class_implements_triple() {
    let facts =
        extract_php_facts("Store.php", "<?php class Store implements Recallable {} ").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "implements".to_string(),
        object: "Interface:Recallable".to_string(),
    }));
}

#[test]
fn extract_php_facts_emit_interface_extends_triple() {
    let facts = extract_php_facts(
        "memory.php",
        "<?php interface Recallable extends BaseRecallable {}",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Interface:Recallable".to_string(),
        predicate: "extends".to_string(),
        object: "Interface:BaseRecallable".to_string(),
    }));
}

#[test]
fn extract_php_namespaces_emit_definition_facts() {
    let source = "<?php namespace Memory\\Core; class Store {}";

    assert_eq!(
        extract_php_namespaces(source).unwrap(),
        vec!["Memory\\Core".to_string()]
    );
    assert!(
        extract_php_facts("Memory.php", source)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "Memory.php".to_string(),
                predicate: "defines".to_string(),
                object: "Namespace:Memory\\Core".to_string(),
            })
    );
}

#[test]
fn extract_swift_facts_emit_class_implements_protocol_triple() {
    let facts = extract_swift_facts(
        "Store.swift",
        "protocol Recallable {}\nclass Store: Recallable {}",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "implements".to_string(),
        object: "Protocol:Recallable".to_string(),
    }));
}

#[test]
fn extract_swift_facts_emit_protocol_extends_triple() {
    let facts =
        extract_swift_facts("Memory.swift", "protocol Recallable: BaseRecallable {}").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Protocol:Recallable".to_string(),
        predicate: "extends".to_string(),
        object: "Protocol:BaseRecallable".to_string(),
    }));
}

#[test]
fn extract_csharp_facts_emit_class_implements_interface_triple() {
    let facts = extract_csharp_facts(
        "Store.cs",
        "interface IRecallable {} class Store : IRecallable {}",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "implements".to_string(),
        object: "Interface:IRecallable".to_string(),
    }));
}

#[test]
fn extract_csharp_facts_emit_record_extends_triple() {
    let facts = extract_csharp_facts("User.cs", "record User : BaseUser {}").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Record:User".to_string(),
        predicate: "extends".to_string(),
        object: "Record:BaseUser".to_string(),
    }));
}

#[test]
fn extract_csharp_facts_emit_struct_implements_interface_triple() {
    let facts = extract_csharp_facts(
        "Store.cs",
        "interface IRecallable {} struct Store : IRecallable {}",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Struct:Store".to_string(),
        predicate: "implements".to_string(),
        object: "Interface:IRecallable".to_string(),
    }));
}

#[test]
fn extract_csharp_facts_emit_record_implements_interface_triple() {
    let facts = extract_csharp_facts(
        "User.cs",
        "interface IRecallable {} record User : IRecallable {}",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Record:User".to_string(),
        predicate: "implements".to_string(),
        object: "Interface:IRecallable".to_string(),
    }));
}

#[test]
fn extract_csharp_facts_emit_interface_extends_triple() {
    let facts =
        extract_csharp_facts("Memory.cs", "interface Recallable : BaseRecallable {}").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Interface:Recallable".to_string(),
        predicate: "extends".to_string(),
        object: "Interface:BaseRecallable".to_string(),
    }));
}

#[test]
fn extract_csharp_namespaces_emit_definition_facts() {
    let source = "namespace Memory.Core { class Store {} }";

    assert_eq!(
        extract_csharp_namespaces(source).unwrap(),
        vec!["Memory.Core".to_string()]
    );
    assert!(
        extract_csharp_facts("Memory.cs", source)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "Memory.cs".to_string(),
                predicate: "defines".to_string(),
                object: "Namespace:Memory.Core".to_string(),
            })
    );
}

#[test]
fn extract_cpp_namespaces_emit_definition_facts() {
    let source = "namespace memory { class Store {}; }";

    assert_eq!(
        extract_cpp_namespaces(source).unwrap(),
        vec!["memory".to_string()]
    );
    assert!(
        extract_cpp_facts("memory.hpp", source)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "memory.hpp".to_string(),
                predicate: "defines".to_string(),
                object: "Namespace:memory".to_string(),
            })
    );
}

#[test]
fn extract_swift_actors_emit_definition_facts() {
    let source = "actor MemoryStore {}";

    assert_eq!(
        extract_swift_actors(source).unwrap(),
        vec!["MemoryStore".to_string()]
    );
    assert!(
        extract_swift_facts("Memory.swift", source)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "Memory.swift".to_string(),
                predicate: "defines".to_string(),
                object: "Actor:MemoryStore".to_string(),
            })
    );
}

#[test]
fn extract_java_annotations_emit_definition_facts() {
    let source = "@interface DurableMemory {}";

    assert_eq!(
        extract_java_annotations(source).unwrap(),
        vec!["DurableMemory".to_string()]
    );
    assert!(
        extract_java_facts("Memory.java", source)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "Memory.java".to_string(),
                predicate: "defines".to_string(),
                object: "Annotation:DurableMemory".to_string(),
            })
    );
}

#[test]
fn extract_java_records_emit_definition_facts() {
    let source = "record MemoryEvent(String kind) {}";

    assert_eq!(
        extract_java_records(source).unwrap(),
        vec!["MemoryEvent".to_string()]
    );
    assert!(
        extract_java_facts("Memory.java", source)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "Memory.java".to_string(),
                predicate: "defines".to_string(),
                object: "Record:MemoryEvent".to_string(),
            })
    );
}

#[test]
fn extract_csharp_records_emit_definition_facts() {
    let source = "record MemoryEvent(string Kind);";

    assert_eq!(
        extract_csharp_records(source).unwrap(),
        vec!["MemoryEvent".to_string()]
    );
    assert!(
        extract_csharp_facts("Memory.cs", source)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "Memory.cs".to_string(),
                predicate: "defines".to_string(),
                object: "Record:MemoryEvent".to_string(),
            })
    );
}

#[test]
fn extract_csharp_delegates_emit_definition_facts() {
    let source = "delegate void RecallHandler(string memory);";

    assert_eq!(
        extract_csharp_delegates(source).unwrap(),
        vec!["RecallHandler".to_string()]
    );
    assert!(
        extract_csharp_facts("Memory.cs", source)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "Memory.cs".to_string(),
                predicate: "defines".to_string(),
                object: "Delegate:RecallHandler".to_string(),
            })
    );
}

#[test]
fn extract_cpp_type_aliases_emit_definition_facts() {
    let source = "using MemoryId = int; typedef struct {} Chunk;";

    assert_eq!(
        extract_cpp_type_aliases(source).unwrap(),
        vec!["MemoryId".to_string(), "Chunk".to_string()]
    );
    let facts = extract_cpp_facts("memory.hpp", source).unwrap();
    assert!(facts.contains(&ExtractedFact {
        subject: "memory.hpp".to_string(),
        predicate: "defines".to_string(),
        object: "TypeAlias:MemoryId".to_string(),
    }));
    assert!(facts.contains(&ExtractedFact {
        subject: "memory.hpp".to_string(),
        predicate: "defines".to_string(),
        object: "TypeAlias:Chunk".to_string(),
    }));
}

#[test]
fn extract_c_typedef_aliases_emit_definition_facts() {
    let source = "typedef struct {} Chunk; typedef int MemoryId;";

    assert_eq!(
        extract_c_type_aliases(source).unwrap(),
        vec!["Chunk".to_string(), "MemoryId".to_string()]
    );
    let facts = extract_c_facts("memory.h", source).unwrap();
    assert!(facts.contains(&ExtractedFact {
        subject: "memory.h".to_string(),
        predicate: "defines".to_string(),
        object: "TypeAlias:Chunk".to_string(),
    }));
    assert!(facts.contains(&ExtractedFact {
        subject: "memory.h".to_string(),
        predicate: "defines".to_string(),
        object: "TypeAlias:MemoryId".to_string(),
    }));
}

#[test]
fn extract_c_struct_and_enum_symbols_emit_definition_facts() {
    let source = "struct Chunk {}; enum MemoryKind { EPISODIC };";

    assert_eq!(
        extract_c_structs(source).unwrap(),
        vec!["Chunk".to_string()]
    );
    assert_eq!(
        extract_c_enums(source).unwrap(),
        vec!["MemoryKind".to_string()]
    );
    let facts = extract_c_facts("memory.c", source).unwrap();
    assert!(facts.contains(&ExtractedFact {
        subject: "memory.c".to_string(),
        predicate: "defines".to_string(),
        object: "Struct:Chunk".to_string(),
    }));
    assert!(facts.contains(&ExtractedFact {
        subject: "memory.c".to_string(),
        predicate: "defines".to_string(),
        object: "Enum:MemoryKind".to_string(),
    }));
}

#[test]
fn extract_common_language_type_symbols_emit_definition_facts() {
    let java = "enum MemoryKind { EPISODIC }";
    assert_eq!(
        extract_java_enums(java).unwrap(),
        vec!["MemoryKind".to_string()]
    );
    assert!(
        extract_java_facts("MemoryKind.java", java)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "MemoryKind.java".to_string(),
                predicate: "defines".to_string(),
                object: "Enum:MemoryKind".to_string(),
            })
    );

    let cpp = "struct Chunk {}; enum MemoryKind { Episodic };";
    assert_eq!(extract_cpp_structs(cpp).unwrap(), vec!["Chunk".to_string()]);
    assert_eq!(
        extract_cpp_enums(cpp).unwrap(),
        vec!["MemoryKind".to_string()]
    );
    assert!(
        extract_cpp_facts("memory.cpp", cpp)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "memory.cpp".to_string(),
                predicate: "defines".to_string(),
                object: "Enum:MemoryKind".to_string(),
            })
    );

    let csharp = "interface Recallable {} struct Chunk {} enum MemoryKind { Episodic }";
    assert_eq!(
        extract_csharp_interfaces(csharp).unwrap(),
        vec!["Recallable".to_string()]
    );
    assert_eq!(
        extract_csharp_structs(csharp).unwrap(),
        vec!["Chunk".to_string()]
    );
    assert_eq!(
        extract_csharp_enums(csharp).unwrap(),
        vec!["MemoryKind".to_string()]
    );
    assert!(
        extract_csharp_facts("Memory.cs", csharp)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "Memory.cs".to_string(),
                predicate: "defines".to_string(),
                object: "Interface:Recallable".to_string(),
            })
    );

    let php = "<?php interface Recallable {} enum MemoryKind { case Episodic; }";
    assert_eq!(
        extract_php_interfaces(php).unwrap(),
        vec!["Recallable".to_string()]
    );
    assert_eq!(
        extract_php_enums(php).unwrap(),
        vec!["MemoryKind".to_string()]
    );
    assert!(
        extract_php_facts("memory.php", php)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "memory.php".to_string(),
                predicate: "defines".to_string(),
                object: "Enum:MemoryKind".to_string(),
            })
    );

    let swift = "protocol Recallable {} struct Chunk {} enum MemoryKind { case episodic }";
    assert_eq!(
        extract_swift_protocols(swift).unwrap(),
        vec!["Recallable".to_string()]
    );
    assert_eq!(
        extract_swift_structs(swift).unwrap(),
        vec!["Chunk".to_string()]
    );
    assert_eq!(
        extract_swift_enums(swift).unwrap(),
        vec!["MemoryKind".to_string()]
    );
    assert!(
        extract_swift_facts("Memory.swift", swift)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "Memory.swift".to_string(),
                predicate: "defines".to_string(),
                object: "Protocol:Recallable".to_string(),
            })
    );
}

#[test]
fn extract_ruby_facts_emit_class_extends_triple() {
    let facts = extract_ruby_facts("store.rb", "class Store < BaseStore\nend").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Class:BaseStore".to_string(),
    }));
}

#[test]
fn extract_php_facts_emit_class_uses_trait_triple() {
    let facts = extract_php_facts(
        "Store.php",
        "<?php trait RecordsMemory {} class Store { use RecordsMemory; }",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "uses".to_string(),
        object: "Trait:RecordsMemory".to_string(),
    }));
}

#[test]
fn extract_ruby_facts_emit_class_includes_module_triple() {
    let facts = extract_ruby_facts(
        "store.rb",
        "module Recallable\nend\nclass Store\n  include Recallable\nend",
    )
    .unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "implements".to_string(),
        object: "Module:Recallable".to_string(),
    }));
}

#[test]
fn extract_ruby_modules_and_php_traits_emit_definition_facts() {
    let ruby = "module Memory\nend";
    assert_eq!(
        extract_ruby_modules(ruby).unwrap(),
        vec!["Memory".to_string()]
    );
    assert!(
        extract_ruby_facts("memory.rb", ruby)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "memory.rb".to_string(),
                predicate: "defines".to_string(),
                object: "Module:Memory".to_string(),
            })
    );

    let php = "<?php trait RecordsMemory {}";
    assert_eq!(
        extract_php_traits(php).unwrap(),
        vec!["RecordsMemory".to_string()]
    );
    assert!(
        extract_php_facts("memory.php", php)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "memory.php".to_string(),
                predicate: "defines".to_string(),
                object: "Trait:RecordsMemory".to_string(),
            })
    );
}

#[test]
fn extract_kotlin_facts_emit_class_implements_interface_triple() {
    let facts =
        extract_kotlin_facts("Store.kt", "interface Recallable\nclass Store : Recallable").unwrap();

    assert!(facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "implements".to_string(),
        object: "Interface:Recallable".to_string(),
    }));
}

#[test]
fn extract_kotlin_facts_emit_delegation_extends_triples() {
    let class_facts = extract_kotlin_facts("Store.kt", "class Store : BaseStore()").unwrap();
    assert!(class_facts.contains(&ExtractedFact {
        subject: "Class:Store".to_string(),
        predicate: "extends".to_string(),
        object: "Class:BaseStore".to_string(),
    }));

    let interface_facts =
        extract_kotlin_facts("Recallable.kt", "interface Recallable : BaseRecallable").unwrap();
    assert!(interface_facts.contains(&ExtractedFact {
        subject: "Interface:Recallable".to_string(),
        predicate: "extends".to_string(),
        object: "Interface:BaseRecallable".to_string(),
    }));
}

#[test]
fn extract_kotlin_interface_and_enum_symbols_emit_definition_facts() {
    let source = "interface Recallable\nenum class MemoryKind { EPISODIC }";

    assert_eq!(
        extract_kotlin_interfaces(source).unwrap(),
        vec!["Recallable".to_string()]
    );
    assert_eq!(
        extract_kotlin_enums(source).unwrap(),
        vec!["MemoryKind".to_string()]
    );
    assert!(
        extract_kotlin_facts("Memory.kt", source)
            .unwrap()
            .contains(&ExtractedFact {
                subject: "Memory.kt".to_string(),
                predicate: "defines".to_string(),
                object: "Enum:MemoryKind".to_string(),
            })
    );
}

#[test]
fn extract_facts_for_path_dispatches_common_language_extensions() {
    let cases = [
        ("src/lib.rs", "fn remember() {}", "Function:remember"),
        ("agent.py", "def recall():\n    pass\n", "Function:recall"),
        ("store.ts", "class Store {}", "Class:Store"),
        ("store.js", "class Store {}", "Class:Store"),
        (
            "memory.go",
            "package memory\nfunc Remember() {}",
            "Function:Remember",
        ),
        (
            "Memory.java",
            "interface Recallable {}",
            "Interface:Recallable",
        ),
        (
            "memory.c",
            "int remember(void) { return 1; }",
            "Function:remember",
        ),
        ("memory.cpp", "struct Chunk {};", "Struct:Chunk"),
        (
            "Memory.cs",
            "enum MemoryKind { Episodic }",
            "Enum:MemoryKind",
        ),
        ("store.rb", "class Store\nend", "Class:Store"),
        (
            "store.php",
            "<?php interface Recallable {}",
            "Interface:Recallable",
        ),
        ("Store.kt", "class Store", "Class:Store"),
        (
            "Store.swift",
            "protocol Recallable {}",
            "Protocol:Recallable",
        ),
    ];

    for (path, source, object) in cases {
        assert!(
            extract_facts_for_path(path, source)
                .unwrap()
                .contains(&ExtractedFact {
                    subject: path.to_string(),
                    predicate: "defines".to_string(),
                    object: object.to_string(),
                }),
            "expected {path} to emit {object}"
        );
    }

    assert!(
        extract_facts_for_path("README.md", "# Notes")
            .unwrap()
            .is_empty()
    );
}
