//! Deterministic source extractors (ADR-0007).

pub mod bridge;
pub mod lang;

use std::path::Path;

use tree_sitter::{Language, Node, Parser};

pub use lang::c::*;
pub use lang::cpp::*;
pub use lang::csharp::*;
pub use lang::go::*;
pub use lang::java::*;
pub use lang::javascript::*;
pub use lang::kotlin::*;
pub use lang::php::*;
pub use lang::python::*;
pub use lang::ruby::*;
pub use lang::rust::*;
pub use lang::swift::*;
pub use lang::typescript::*;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedFact {
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

/// Language names returned by [`language_for_path`], in dispatch order.
pub const LANGUAGES: &[&str] = &[
    "rust",
    "python",
    "typescript",
    "javascript",
    "go",
    "java",
    "c",
    "cpp",
    "csharp",
    "ruby",
    "php",
    "kotlin",
    "swift",
];

/// The extractor language for `path`, chosen by file extension, or `None`
/// when no bundled grammar handles it.
pub fn language_for_path(path: &str) -> Option<&'static str> {
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" => Some("javascript"),
        "go" => Some("go"),
        "java" => Some("java"),
        "c" | "h" => Some("c"),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Some("cpp"),
        "cs" => Some("csharp"),
        "rb" => Some("ruby"),
        "php" => Some("php"),
        "kt" | "kts" => Some("kotlin"),
        "swift" => Some("swift"),
        _ => None,
    }
}

pub fn extract_facts_for_path(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    match language_for_path(path) {
        Some("rust") => extract_rust_facts(path, source),
        Some("python") => extract_python_facts(path, source),
        Some("typescript") => extract_typescript_facts(path, source),
        Some("javascript") => extract_javascript_facts(path, source),
        Some("go") => extract_go_facts(path, source),
        Some("java") => extract_java_facts(path, source),
        Some("c") => extract_c_facts(path, source),
        Some("cpp") => extract_cpp_facts(path, source),
        Some("csharp") => extract_csharp_facts(path, source),
        Some("ruby") => extract_ruby_facts(path, source),
        Some("php") => extract_php_facts(path, source),
        Some("kotlin") => extract_kotlin_facts(path, source),
        Some("swift") => extract_swift_facts(path, source),
        _ => Ok(Vec::new()),
    }
}

pub(crate) fn parse_with_language(
    source: &str,
    language: Language,
) -> Result<tree_sitter::Tree, Error> {
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    parser.parse(source, None).ok_or(Error::ParseFailed)
}

/// Returns the first direct child with the given kind, without descending into
/// nested nodes. Use this to anchor clause lookups (base lists, heritage
/// clauses) so a nested type's clause is never attributed to an outer type.
pub(crate) fn named_child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
}

pub(crate) fn collect_named_nodes(
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

pub(crate) fn collect_names_from_kinds(
    node: Node<'_>,
    source: &[u8],
    kinds: &[&str],
) -> Result<Vec<String>, Error> {
    let mut names = Vec::new();
    collect_named_nodes(node, source, kinds, &mut names)?;
    Ok(names)
}

/// Collects names like [`collect_names_from_kinds`] but only for nodes that
/// also have the given child field — e.g. requiring a `body` field so C/C++
/// forward declarations and usages (`struct Foo *p;`) are not reported as
/// definitions.
pub(crate) fn collect_names_from_kinds_requiring_field(
    node: Node<'_>,
    source: &[u8],
    kinds: &[&str],
    required_field: &str,
) -> Result<Vec<String>, Error> {
    let mut names = Vec::new();
    collect_named_nodes_requiring_field(node, source, kinds, required_field, &mut names)?;
    Ok(names)
}

fn collect_named_nodes_requiring_field(
    node: Node<'_>,
    source: &[u8],
    kinds: &[&str],
    required_field: &str,
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if kinds.contains(&node.kind())
        && node.child_by_field_name(required_field).is_some()
        && let Some(name) = node.child_by_field_name("name")
    {
        names.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_named_nodes_requiring_field(child, source, kinds, required_field, names)?;
    }
    Ok(())
}

pub(crate) fn definition_facts(path: &str, kind: &str, names: Vec<String>) -> Vec<ExtractedFact> {
    names
        .into_iter()
        .map(|name| ExtractedFact {
            subject: path.to_string(),
            predicate: "defines".to_string(),
            object: format!("{kind}:{name}"),
        })
        .collect()
}

pub(crate) fn collect_descendant_names_from_kinds(
    node: Node<'_>,
    source: &[u8],
    kinds: &[&str],
    descendant_kind: &str,
) -> Result<Vec<String>, Error> {
    let mut names = Vec::new();
    collect_first_descendant_names(node, source, kinds, descendant_kind, &mut names)?;
    Ok(names)
}

fn collect_first_descendant_names(
    node: Node<'_>,
    source: &[u8],
    kinds: &[&str],
    descendant_kind: &str,
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if kinds.contains(&node.kind())
        && let Some(name) = first_named_descendant_of_kind(node, descendant_kind)
    {
        names.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_first_descendant_names(child, source, kinds, descendant_kind, names)?;
    }
    Ok(())
}

pub(crate) fn collect_names_from_kinds_with_field_text(
    node: Node<'_>,
    source: &[u8],
    kinds: &[&str],
    field_name: &str,
    field_text: &str,
) -> Result<Vec<String>, Error> {
    let mut names = Vec::new();
    collect_names_matching_field_text(node, source, kinds, field_name, field_text, &mut names)?;
    Ok(names)
}

fn collect_names_matching_field_text(
    node: Node<'_>,
    source: &[u8],
    kinds: &[&str],
    field_name: &str,
    field_text: &str,
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if kinds.contains(&node.kind())
        && node
            .child_by_field_name(field_name)
            .is_some_and(|field| field.utf8_text(source).is_ok_and(|text| text == field_text))
        && let Some(name) = node.child_by_field_name("name")
    {
        names.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_names_matching_field_text(child, source, kinds, field_name, field_text, names)?;
    }
    Ok(())
}

pub(crate) fn collect_type_definition_aliases(
    node: Node<'_>,
    source: &[u8],
    aliases: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "type_definition" {
        let mut cursor = node.walk();
        for child in node.children_by_field_name("declarator", &mut cursor) {
            if let Some(name) = first_named_descendant_of_kind(child, "type_identifier")
                .or_else(|| first_named_descendant_of_kind(child, "identifier"))
            {
                aliases.push(name.utf8_text(source)?.to_string());
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_type_definition_aliases(child, source, aliases)?;
    }
    Ok(())
}

pub(crate) fn collect_function_variable_names(
    node: Node<'_>,
    source: &[u8],
    functions: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "variable_declarator"
        && let Some(value) = node.child_by_field_name("value")
        && matches!(
            value.kind(),
            "arrow_function" | "function" | "function_expression"
        )
        && let Some(name) = node.child_by_field_name("name")
        && name.kind() == "identifier"
    {
        functions.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_function_variable_names(child, source, functions)?;
    }
    Ok(())
}

pub(crate) fn collect_c_style_function_names(
    node: Node<'_>,
    source: &[u8],
    functions: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "function_definition"
        && let Some(declarator) = node.child_by_field_name("declarator")
        && let Some(name) = first_named_descendant_of_kind(declarator, "identifier")
    {
        functions.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_c_style_function_names(child, source, functions)?;
    }
    Ok(())
}

pub(crate) fn collect_descendant_texts(
    node: Node<'_>,
    source: &[u8],
    kinds: &[&str],
    texts: &mut Vec<String>,
) -> Result<(), Error> {
    if kinds.contains(&node.kind()) {
        texts.push(node.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_descendant_texts(child, source, kinds, texts)?;
    }
    Ok(())
}

pub(crate) fn first_named_descendant_of_kind<'tree>(
    node: Node<'tree>,
    kind: &str,
) -> Option<Node<'tree>> {
    if node.kind() == kind {
        return Some(node);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = first_named_descendant_of_kind(child, kind) {
            return Some(found);
        }
    }
    None
}

pub(crate) fn collect_heritage_type_name(
    node: Node<'_>,
    source: &[u8],
    out: &mut Option<String>,
) -> Result<(), Error> {
    if out.is_some() {
        return Ok(());
    }

    match node.kind() {
        "identifier"
        | "type_identifier"
        | "nested_type_identifier"
        | "member_expression"
        | "scoped_type_identifier" => {
            *out = Some(node.utf8_text(source)?.to_string());
            return Ok(());
        }
        "generic_type" => {
            if let Some(name) = first_named_descendant_of_kind(node, "type_identifier")
                .or_else(|| first_named_descendant_of_kind(node, "identifier"))
            {
                *out = Some(name.utf8_text(source)?.to_string());
            }
            return Ok(());
        }
        "type_arguments" => return Ok(()),
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_heritage_type_name(child, source, out)?;
        if out.is_some() {
            break;
        }
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
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
