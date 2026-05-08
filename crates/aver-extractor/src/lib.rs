//! Deterministic source extractors (ADR-0007).

pub mod prose;

use std::collections::HashSet;

use tree_sitter::{Language, Node, Parser};

pub use prose::parse_prose_facts;

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ExtractedFact {
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

pub fn extract_rust_functions(source: &str) -> Result<Vec<String>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut functions = Vec::new();
    collect_function_names(tree.root_node(), source.as_bytes(), &mut functions)?;
    Ok(functions)
}

pub fn extract_rust_imports(source: &str) -> Result<Vec<String>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut imports = Vec::new();
    collect_imports(tree.root_node(), source.as_bytes(), &mut imports)?;
    Ok(imports)
}

pub fn extract_rust_calls(source: &str) -> Result<Vec<String>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut calls = Vec::new();
    collect_calls(tree.root_node(), source.as_bytes(), &mut calls)?;
    Ok(calls)
}

pub fn extract_rust_structs(source: &str) -> Result<Vec<String>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut structs = Vec::new();
    collect_structs(tree.root_node(), source.as_bytes(), &mut structs)?;
    Ok(structs)
}

pub fn extract_rust_enums(source: &str) -> Result<Vec<String>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut enums = Vec::new();
    collect_enums(tree.root_node(), source.as_bytes(), &mut enums)?;
    Ok(enums)
}

pub fn extract_rust_traits(source: &str) -> Result<Vec<String>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut traits = Vec::new();
    collect_traits(tree.root_node(), source.as_bytes(), &mut traits)?;
    Ok(traits)
}

pub fn extract_rust_consts(source: &str) -> Result<Vec<String>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut consts = Vec::new();
    collect_consts(tree.root_node(), source.as_bytes(), &mut consts)?;
    Ok(consts)
}

pub fn extract_rust_modules(source: &str) -> Result<Vec<String>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut modules = Vec::new();
    collect_modules(tree.root_node(), source.as_bytes(), &mut modules)?;
    Ok(modules)
}

pub fn extract_rust_tests(source: &str) -> Result<Vec<String>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut tests = Vec::new();
    collect_tests(tree.root_node(), source.as_bytes(), &mut tests)?;
    Ok(tests)
}

pub fn map_rust_tests_to_functions(source: &str) -> Result<Vec<(String, String)>, Error> {
    let tests = extract_rust_tests(source)?;
    let test_names = tests.iter().cloned().collect::<HashSet<_>>();
    let functions = extract_rust_functions(source)?
        .into_iter()
        .filter(|function| !test_names.contains(function))
        .collect::<Vec<_>>();

    let mut mappings = Vec::new();
    for test in tests {
        if let Some(function) = functions
            .iter()
            .filter(|function| test.starts_with(&format!("{function}_")))
            .max_by_key(|function| function.len())
        {
            mappings.push((test, function.clone()));
        }
    }
    Ok(mappings)
}

pub fn extract_python_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_python::language())?;
    let mut functions = Vec::new();
    collect_named_nodes(
        tree.root_node(),
        source.as_bytes(),
        &["function_definition"],
        &mut functions,
    )?;
    Ok(functions)
}

pub fn extract_python_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    Ok(extract_python_functions(source)?
        .into_iter()
        .map(|function| ExtractedFact {
            subject: path.to_string(),
            predicate: "defines".to_string(),
            object: format!("Function:{function}"),
        })
        .collect())
}

pub fn extract_typescript_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_typescript::language_typescript())?;
    let mut functions = Vec::new();
    collect_named_nodes(
        tree.root_node(),
        source.as_bytes(),
        &["function_declaration", "method_definition"],
        &mut functions,
    )?;
    Ok(functions)
}

pub fn extract_typescript_classes(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_typescript::language_typescript())?;
    let mut classes = Vec::new();
    collect_named_nodes(
        tree.root_node(),
        source.as_bytes(),
        &["class_declaration"],
        &mut classes,
    )?;
    Ok(classes)
}

pub fn extract_typescript_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = extract_typescript_functions(source)?
        .into_iter()
        .map(|function| ExtractedFact {
            subject: path.to_string(),
            predicate: "defines".to_string(),
            object: format!("Function:{function}"),
        })
        .collect::<Vec<_>>();
    facts.extend(
        extract_typescript_classes(source)?
            .into_iter()
            .map(|class_name| ExtractedFact {
                subject: path.to_string(),
                predicate: "defines".to_string(),
                object: format!("Class:{class_name}"),
            }),
    );
    facts.extend(extract_typescript_extends_facts(source));
    Ok(facts)
}

pub fn extract_go_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_go::language())?;
    let mut functions = Vec::new();
    collect_named_nodes(
        tree.root_node(),
        source.as_bytes(),
        &["function_declaration", "method_declaration"],
        &mut functions,
    )?;
    Ok(functions)
}

pub fn extract_go_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    Ok(extract_go_functions(source)?
        .into_iter()
        .map(|function| ExtractedFact {
            subject: path.to_string(),
            predicate: "defines".to_string(),
            object: format!("Function:{function}"),
        })
        .collect())
}

pub fn extract_rust_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = extract_rust_functions(source)?
        .into_iter()
        .map(|function| ExtractedFact {
            subject: path.to_string(),
            predicate: "defines".to_string(),
            object: format!("Function:{function}"),
        })
        .collect::<Vec<_>>();

    facts.extend(
        extract_rust_structs(source)?
            .into_iter()
            .map(|struct_name| ExtractedFact {
                subject: path.to_string(),
                predicate: "defines".to_string(),
                object: format!("Struct:{struct_name}"),
            }),
    );
    facts.extend(
        extract_rust_enums(source)?
            .into_iter()
            .map(|enum_name| ExtractedFact {
                subject: path.to_string(),
                predicate: "defines".to_string(),
                object: format!("Enum:{enum_name}"),
            }),
    );
    facts.extend(extract_rust_enum_variant_facts(source)?);
    facts.extend(
        extract_rust_traits(source)?
            .into_iter()
            .map(|trait_name| ExtractedFact {
                subject: path.to_string(),
                predicate: "defines".to_string(),
                object: format!("Trait:{trait_name}"),
            }),
    );
    facts.extend(
        extract_rust_consts(source)?
            .into_iter()
            .map(|const_name| ExtractedFact {
                subject: path.to_string(),
                predicate: "defines".to_string(),
                object: format!("Const:{const_name}"),
            }),
    );
    facts.extend(extract_rust_module_definition_facts(path, source)?);
    facts.extend(extract_rust_module_import_facts(source)?);
    facts.extend(extract_rust_module_trait_facts(source)?);
    facts.extend(extract_rust_module_struct_facts(source)?);
    facts.extend(extract_rust_module_enum_facts(source)?);
    facts.extend(extract_rust_module_const_facts(source)?);
    facts.extend(
        extract_rust_imports(source)?
            .into_iter()
            .map(|module| ExtractedFact {
                subject: path.to_string(),
                predicate: "imports".to_string(),
                object: format!("Module:{module}"),
            }),
    );
    facts.extend(extract_rust_function_call_facts(source)?);
    facts.extend(extract_rust_module_function_facts(source)?);
    facts.extend(extract_rust_module_impl_method_facts(source)?);
    facts.extend(extract_rust_impl_method_facts(source)?);
    facts.extend(extract_rust_impl_method_call_facts(source)?);
    facts.extend(extract_rust_module_impl_trait_facts(source)?);
    facts.extend(extract_rust_impl_trait_facts(source)?);
    facts.extend(extract_rust_module_test_mapping_facts(source)?);
    facts.extend(
        map_rust_tests_to_functions(source)?
            .into_iter()
            .map(|(test, function)| ExtractedFact {
                subject: format!("Function:{test}"),
                predicate: "tests".to_string(),
                object: format!("Function:{function}"),
            }),
    );
    Ok(facts)
}

fn extract_rust_function_call_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut facts = Vec::new();
    collect_function_call_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

fn extract_rust_module_definition_facts(
    path: &str,
    source: &str,
) -> Result<Vec<ExtractedFact>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut facts = Vec::new();
    collect_module_definition_facts(tree.root_node(), source.as_bytes(), path, "", &mut facts)?;
    Ok(facts)
}

fn extract_rust_module_trait_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut facts = Vec::new();
    collect_module_trait_facts(tree.root_node(), source.as_bytes(), "", &mut facts)?;
    Ok(facts)
}

fn extract_rust_module_import_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut facts = Vec::new();
    collect_module_import_facts(tree.root_node(), source.as_bytes(), "", &mut facts)?;
    Ok(facts)
}

fn extract_rust_module_struct_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut facts = Vec::new();
    collect_module_struct_facts(tree.root_node(), source.as_bytes(), "", &mut facts)?;
    Ok(facts)
}

fn extract_rust_module_enum_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut facts = Vec::new();
    collect_module_enum_facts(tree.root_node(), source.as_bytes(), "", &mut facts)?;
    Ok(facts)
}

fn extract_rust_module_const_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut facts = Vec::new();
    collect_module_const_facts(tree.root_node(), source.as_bytes(), "", &mut facts)?;
    Ok(facts)
}

fn extract_rust_enum_variant_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut facts = Vec::new();
    collect_enum_variant_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

fn extract_rust_impl_method_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut facts = Vec::new();
    collect_impl_method_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

fn extract_rust_module_impl_method_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut facts = Vec::new();
    collect_module_impl_method_facts(tree.root_node(), source.as_bytes(), "", &mut facts)?;
    Ok(facts)
}

fn extract_rust_module_function_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut facts = Vec::new();
    collect_module_function_facts(tree.root_node(), source.as_bytes(), "", &mut facts)?;
    Ok(facts)
}

fn extract_rust_impl_method_call_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut facts = Vec::new();
    collect_impl_method_call_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

fn extract_rust_impl_trait_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut facts = Vec::new();
    collect_impl_trait_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

fn extract_rust_module_impl_trait_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut facts = Vec::new();
    collect_module_impl_trait_facts(tree.root_node(), source.as_bytes(), "", &mut facts)?;
    Ok(facts)
}

fn extract_rust_module_test_mapping_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_rust::language())?;
    let tree = parser.parse(source, None).ok_or(Error::ParseFailed)?;

    let mut facts = Vec::new();
    collect_module_test_mapping_facts(tree.root_node(), source.as_bytes(), "", &mut facts)?;
    Ok(facts)
}

fn parse_with_language(source: &str, language: Language) -> Result<tree_sitter::Tree, Error> {
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    parser.parse(source, None).ok_or(Error::ParseFailed)
}

fn collect_named_nodes(
    node: Node<'_>,
    source: &[u8],
    kinds: &[&str],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if kinds.contains(&node.kind())
        && let Some(name) = node.child_by_field_name("name")
    {
        names.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_named_nodes(child, source, kinds, names)?;
    }
    Ok(())
}

fn extract_typescript_extends_facts(source: &str) -> Vec<ExtractedFact> {
    source
        .lines()
        .filter_map(|line| {
            let mut tokens = line
                .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'))
                .filter(|token| !token.is_empty());
            while let Some(token) = tokens.next() {
                if token == "class" {
                    let class_name = tokens.next()?;
                    if tokens.next()? == "extends" {
                        let base = tokens.next()?;
                        return Some(ExtractedFact {
                            subject: format!("Class:{class_name}"),
                            predicate: "extends".to_string(),
                            object: format!("Class:{base}"),
                        });
                    }
                }
            }
            None
        })
        .collect()
}

fn collect_function_names(
    node: Node<'_>,
    source: &[u8],
    functions: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "function_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        functions.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_function_names(child, source, functions)?;
    }
    Ok(())
}

fn collect_imports(node: Node<'_>, source: &[u8], imports: &mut Vec<String>) -> Result<(), Error> {
    if node.kind() == "use_declaration" {
        let text = node.utf8_text(source)?;
        imports.push(
            text.trim()
                .trim_start_matches("use ")
                .trim_end_matches(';')
                .to_string(),
        );
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_imports(child, source, imports)?;
    }
    Ok(())
}

fn collect_calls(node: Node<'_>, source: &[u8], calls: &mut Vec<String>) -> Result<(), Error> {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
    {
        calls.push(function.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls(child, source, calls)?;
    }
    Ok(())
}

fn collect_structs(node: Node<'_>, source: &[u8], structs: &mut Vec<String>) -> Result<(), Error> {
    if node.kind() == "struct_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        structs.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_structs(child, source, structs)?;
    }
    Ok(())
}

fn collect_enums(node: Node<'_>, source: &[u8], enums: &mut Vec<String>) -> Result<(), Error> {
    if node.kind() == "enum_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        enums.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_enums(child, source, enums)?;
    }
    Ok(())
}

fn collect_traits(node: Node<'_>, source: &[u8], traits: &mut Vec<String>) -> Result<(), Error> {
    if node.kind() == "trait_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        traits.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_traits(child, source, traits)?;
    }
    Ok(())
}

fn collect_modules(node: Node<'_>, source: &[u8], modules: &mut Vec<String>) -> Result<(), Error> {
    if node.kind() == "mod_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        modules.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_modules(child, source, modules)?;
    }
    Ok(())
}

fn collect_consts(node: Node<'_>, source: &[u8], consts: &mut Vec<String>) -> Result<(), Error> {
    if node.kind() == "const_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        consts.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_consts(child, source, consts)?;
    }
    Ok(())
}

fn collect_tests(node: Node<'_>, source: &[u8], tests: &mut Vec<String>) -> Result<(), Error> {
    let mut cursor = node.walk();
    let mut preceding_test_attr = false;
    for child in node.children(&mut cursor) {
        if child.kind() == "attribute_item" {
            preceding_test_attr = child.utf8_text(source)?.contains("#[test]");
            continue;
        }

        if preceding_test_attr
            && child.kind() == "function_item"
            && let Some(name) = child.child_by_field_name("name")
        {
            tests.push(name.utf8_text(source)?.to_string());
        }

        collect_tests(child, source, tests)?;
        preceding_test_attr = false;
    }
    Ok(())
}

fn collect_function_call_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "function_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let caller = name.utf8_text(source)?.to_string();
        let mut calls = Vec::new();
        collect_calls(node, source, &mut calls)?;
        facts.extend(calls.into_iter().map(|callee| ExtractedFact {
            subject: format!("Function:{caller}"),
            predicate: "calls".to_string(),
            object: format!("Function:{callee}"),
        }));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_function_call_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_module_function_facts(
    node: Node<'_>,
    source: &[u8],
    module_path: &str,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "mod_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let module_name = name.utf8_text(source)?;
        let nested_path = if module_path.is_empty() {
            module_name.to_string()
        } else {
            format!("{module_path}::{module_name}")
        };

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_module_function_facts(child, source, &nested_path, facts)?;
        }
        return Ok(());
    }

    if !module_path.is_empty()
        && node.kind() == "function_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let function = name.utf8_text(source)?;
        facts.push(ExtractedFact {
            subject: format!("Module:{module_path}"),
            predicate: "defines".to_string(),
            object: format!("Function:{module_path}::{function}"),
        });
        let mut calls = Vec::new();
        collect_calls(node, source, &mut calls)?;
        facts.extend(calls.into_iter().map(|callee| ExtractedFact {
            subject: format!("Function:{module_path}::{function}"),
            predicate: "calls".to_string(),
            object: format!("Function:{}", qualify_module_call(&callee, module_path)),
        }));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_module_function_facts(child, source, module_path, facts)?;
    }
    Ok(())
}

fn qualify_module_call(callee: &str, module_path: &str) -> String {
    if callee.contains("::") || callee.contains('.') {
        callee.to_string()
    } else {
        format!("{module_path}::{callee}")
    }
}

fn collect_module_definition_facts(
    node: Node<'_>,
    source: &[u8],
    file_path: &str,
    module_path: &str,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "mod_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let module_name = name.utf8_text(source)?;
        let nested_path = if module_path.is_empty() {
            module_name.to_string()
        } else {
            format!("{module_path}::{module_name}")
        };
        let subject = if module_path.is_empty() {
            file_path.to_string()
        } else {
            format!("Module:{module_path}")
        };
        facts.push(ExtractedFact {
            subject,
            predicate: "defines".to_string(),
            object: format!("Module:{nested_path}"),
        });

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_module_definition_facts(child, source, file_path, &nested_path, facts)?;
        }
        return Ok(());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_module_definition_facts(child, source, file_path, module_path, facts)?;
    }
    Ok(())
}

fn collect_module_impl_method_facts(
    node: Node<'_>,
    source: &[u8],
    module_path: &str,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "mod_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let module_name = name.utf8_text(source)?;
        let nested_path = if module_path.is_empty() {
            module_name.to_string()
        } else {
            format!("{module_path}::{module_name}")
        };

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_module_impl_method_facts(child, source, &nested_path, facts)?;
        }
        return Ok(());
    }

    if !module_path.is_empty()
        && node.kind() == "impl_item"
        && let Some(type_node) = node.child_by_field_name("type")
    {
        let type_name = qualify_module_type(type_node.utf8_text(source)?, module_path);
        let mut methods = Vec::new();
        collect_function_names(node, source, &mut methods)?;
        facts.extend(methods.into_iter().map(|method| ExtractedFact {
            subject: format!("Type:{type_name}"),
            predicate: "defines".to_string(),
            object: format!("Function:{type_name}::{method}"),
        }));
        collect_qualified_method_call_facts(node, source, &type_name, facts)?;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_module_impl_method_facts(child, source, module_path, facts)?;
    }
    Ok(())
}

fn qualify_module_type(type_name: &str, module_path: &str) -> String {
    if type_name.contains("::") {
        type_name.to_string()
    } else {
        format!("{module_path}::{type_name}")
    }
}

fn collect_module_trait_facts(
    node: Node<'_>,
    source: &[u8],
    module_path: &str,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "mod_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let module_name = name.utf8_text(source)?;
        let nested_path = if module_path.is_empty() {
            module_name.to_string()
        } else {
            format!("{module_path}::{module_name}")
        };

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_module_trait_facts(child, source, &nested_path, facts)?;
        }
        return Ok(());
    }

    if !module_path.is_empty()
        && node.kind() == "trait_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let trait_name = name.utf8_text(source)?;
        facts.push(ExtractedFact {
            subject: format!("Module:{module_path}"),
            predicate: "defines".to_string(),
            object: format!("Trait:{module_path}::{trait_name}"),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_module_trait_facts(child, source, module_path, facts)?;
    }
    Ok(())
}

fn collect_module_struct_facts(
    node: Node<'_>,
    source: &[u8],
    module_path: &str,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "mod_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let module_name = name.utf8_text(source)?;
        let nested_path = if module_path.is_empty() {
            module_name.to_string()
        } else {
            format!("{module_path}::{module_name}")
        };

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_module_struct_facts(child, source, &nested_path, facts)?;
        }
        return Ok(());
    }

    if !module_path.is_empty()
        && node.kind() == "struct_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let struct_name = name.utf8_text(source)?;
        facts.push(ExtractedFact {
            subject: format!("Module:{module_path}"),
            predicate: "defines".to_string(),
            object: format!("Struct:{module_path}::{struct_name}"),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_module_struct_facts(child, source, module_path, facts)?;
    }
    Ok(())
}

fn collect_module_enum_facts(
    node: Node<'_>,
    source: &[u8],
    module_path: &str,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "mod_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let module_name = name.utf8_text(source)?;
        let nested_path = if module_path.is_empty() {
            module_name.to_string()
        } else {
            format!("{module_path}::{module_name}")
        };

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_module_enum_facts(child, source, &nested_path, facts)?;
        }
        return Ok(());
    }

    if !module_path.is_empty()
        && node.kind() == "enum_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let enum_name = name.utf8_text(source)?;
        let qualified_enum = format!("{module_path}::{enum_name}");
        facts.push(ExtractedFact {
            subject: format!("Module:{module_path}"),
            predicate: "defines".to_string(),
            object: format!("Enum:{qualified_enum}"),
        });
        let mut variants = Vec::new();
        collect_enum_variants(node, source, &mut variants)?;
        facts.extend(variants.into_iter().map(|variant| ExtractedFact {
            subject: format!("Enum:{qualified_enum}"),
            predicate: "defines".to_string(),
            object: format!("Variant:{qualified_enum}::{variant}"),
        }));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_module_enum_facts(child, source, module_path, facts)?;
    }
    Ok(())
}

fn collect_enum_variant_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "enum_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let enum_name = name.utf8_text(source)?;
        let mut variants = Vec::new();
        collect_enum_variants(node, source, &mut variants)?;
        facts.extend(variants.into_iter().map(|variant| ExtractedFact {
            subject: format!("Enum:{enum_name}"),
            predicate: "defines".to_string(),
            object: format!("Variant:{enum_name}::{variant}"),
        }));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_enum_variant_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_enum_variants(
    node: Node<'_>,
    source: &[u8],
    variants: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "enum_variant"
        && let Some(name) = node.child_by_field_name("name")
    {
        variants.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_enum_variants(child, source, variants)?;
    }
    Ok(())
}

fn collect_module_const_facts(
    node: Node<'_>,
    source: &[u8],
    module_path: &str,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "mod_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let module_name = name.utf8_text(source)?;
        let nested_path = if module_path.is_empty() {
            module_name.to_string()
        } else {
            format!("{module_path}::{module_name}")
        };

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_module_const_facts(child, source, &nested_path, facts)?;
        }
        return Ok(());
    }

    if !module_path.is_empty()
        && node.kind() == "const_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let const_name = name.utf8_text(source)?;
        facts.push(ExtractedFact {
            subject: format!("Module:{module_path}"),
            predicate: "defines".to_string(),
            object: format!("Const:{module_path}::{const_name}"),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_module_const_facts(child, source, module_path, facts)?;
    }
    Ok(())
}

fn collect_module_import_facts(
    node: Node<'_>,
    source: &[u8],
    module_path: &str,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "mod_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let module_name = name.utf8_text(source)?;
        let nested_path = if module_path.is_empty() {
            module_name.to_string()
        } else {
            format!("{module_path}::{module_name}")
        };

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_module_import_facts(child, source, &nested_path, facts)?;
        }
        return Ok(());
    }

    if !module_path.is_empty() && node.kind() == "use_declaration" {
        facts.push(ExtractedFact {
            subject: format!("Module:{module_path}"),
            predicate: "imports".to_string(),
            object: format!("Module:{}", use_path(node.utf8_text(source)?)),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_module_import_facts(child, source, module_path, facts)?;
    }
    Ok(())
}

fn use_path(text: &str) -> String {
    text.trim()
        .trim_start_matches("use ")
        .trim_end_matches(';')
        .to_string()
}

fn collect_impl_method_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "impl_item"
        && let Some(type_node) = node.child_by_field_name("type")
    {
        let type_name = type_node.utf8_text(source)?.to_string();
        let mut methods = Vec::new();
        collect_function_names(node, source, &mut methods)?;
        facts.extend(methods.into_iter().map(|method| ExtractedFact {
            subject: format!("Type:{type_name}"),
            predicate: "defines".to_string(),
            object: format!("Function:{type_name}::{method}"),
        }));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_impl_method_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_impl_method_call_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "impl_item"
        && let Some(type_node) = node.child_by_field_name("type")
    {
        let type_name = type_node.utf8_text(source)?.to_string();
        collect_qualified_method_call_facts(node, source, &type_name, facts)?;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_impl_method_call_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_qualified_method_call_facts(
    node: Node<'_>,
    source: &[u8],
    type_name: &str,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "function_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let method = name.utf8_text(source)?.to_string();
        let mut calls = Vec::new();
        collect_calls(node, source, &mut calls)?;
        facts.extend(calls.into_iter().map(|callee| ExtractedFact {
            subject: format!("Function:{type_name}::{method}"),
            predicate: "calls".to_string(),
            object: format!("Function:{callee}"),
        }));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_qualified_method_call_facts(child, source, type_name, facts)?;
    }
    Ok(())
}

fn collect_module_impl_trait_facts(
    node: Node<'_>,
    source: &[u8],
    module_path: &str,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "mod_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let module_name = name.utf8_text(source)?;
        let nested_path = if module_path.is_empty() {
            module_name.to_string()
        } else {
            format!("{module_path}::{module_name}")
        };

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_module_impl_trait_facts(child, source, &nested_path, facts)?;
        }
        return Ok(());
    }

    if !module_path.is_empty() && node.kind() == "impl_item" {
        let header = node
            .utf8_text(source)?
            .split('{')
            .next()
            .unwrap_or_default()
            .trim()
            .trim_start_matches("impl ");
        if let Some((trait_name, type_name)) = header.split_once(" for ") {
            facts.push(ExtractedFact {
                subject: format!(
                    "Type:{}",
                    qualify_module_type(type_name.trim(), module_path)
                ),
                predicate: "implements".to_string(),
                object: format!(
                    "Trait:{}",
                    qualify_module_type(trait_name.trim(), module_path)
                ),
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_module_impl_trait_facts(child, source, module_path, facts)?;
    }
    Ok(())
}

fn collect_module_test_mapping_facts(
    node: Node<'_>,
    source: &[u8],
    module_path: &str,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "mod_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        let module_name = name.utf8_text(source)?;
        let nested_path = if module_path.is_empty() {
            module_name.to_string()
        } else {
            format!("{module_path}::{module_name}")
        };

        let mut tests = Vec::new();
        collect_tests(node, source, &mut tests)?;
        let test_names = tests.iter().cloned().collect::<HashSet<_>>();
        let mut functions = Vec::new();
        collect_function_names(node, source, &mut functions)?;
        let functions = functions
            .into_iter()
            .filter(|function| !test_names.contains(function))
            .collect::<Vec<_>>();
        for test in tests {
            if let Some(function) = functions
                .iter()
                .filter(|function| test.starts_with(&format!("{function}_")))
                .max_by_key(|function| function.len())
            {
                facts.push(ExtractedFact {
                    subject: format!("Function:{nested_path}::{test}"),
                    predicate: "tests".to_string(),
                    object: format!("Function:{nested_path}::{function}"),
                });
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_module_test_mapping_facts(child, source, &nested_path, facts)?;
        }
        return Ok(());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_module_test_mapping_facts(child, source, module_path, facts)?;
    }
    Ok(())
}

fn collect_impl_trait_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "impl_item" {
        let header = node
            .utf8_text(source)?
            .split('{')
            .next()
            .unwrap_or_default()
            .trim()
            .trim_start_matches("impl ");
        if let Some((trait_name, type_name)) = header.split_once(" for ") {
            facts.push(ExtractedFact {
                subject: format!("Type:{}", type_name.trim()),
                predicate: "implements".to_string(),
                object: format!("Trait:{}", trait_name.trim()),
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_impl_trait_facts(child, source, facts)?;
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("tree-sitter language: {0}")]
    Language(#[from] tree_sitter::LanguageError),
    #[error("tree-sitter parse failed")]
    ParseFailed,
    #[error("utf8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid extracted fact field: {0}")]
    InvalidFact(&'static str),
}
