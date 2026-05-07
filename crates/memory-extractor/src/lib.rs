//! Deterministic source extractors (ADR-0007).

use tree_sitter::{Node, Parser};

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

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("tree-sitter language: {0}")]
    Language(#[from] tree_sitter::LanguageError),
    #[error("tree-sitter parse failed")]
    ParseFailed,
    #[error("utf8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
}
