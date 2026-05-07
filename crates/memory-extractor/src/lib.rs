//! Deterministic source extractors (ADR-0007).

use std::collections::HashSet;

use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq, Eq)]
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
        extract_rust_imports(source)?
            .into_iter()
            .map(|module| ExtractedFact {
                subject: path.to_string(),
                predicate: "imports".to_string(),
                object: format!("Module:{module}"),
            }),
    );
    facts.extend(extract_rust_function_call_facts(source)?);
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

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("tree-sitter language: {0}")]
    Language(#[from] tree_sitter::LanguageError),
    #[error("tree-sitter parse failed")]
    ParseFailed,
    #[error("utf8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
}
