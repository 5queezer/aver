//! Deterministic source extractors (ADR-0007).

pub mod lang;
pub mod prose;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

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
pub use prose::parse_prose_facts;

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ExtractedFact {
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PluginRequest {
    pub id: u64,
    pub method: String,
    pub text: String,
}

#[derive(Debug, serde::Deserialize)]
struct JsonRpcPluginResponse {
    result: ProseExtractionResult,
}

#[derive(Debug, serde::Deserialize)]
struct ProseExtractionResult {
    facts: Vec<ExtractedFact>,
}

#[derive(Debug, Clone)]
pub struct JsonRpcPluginRunner {
    program: String,
    args: Vec<String>,
}

impl JsonRpcPluginRunner {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn extract(&self, request: PluginRequest) -> Result<Vec<ExtractedFact>, Error> {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": request.id,
            "method": request.method,
            "params": { "text": request.text },
        });
        {
            let stdin = child.stdin.as_mut().ok_or(Error::PluginMissingStdin)?;
            writeln!(stdin, "{request}")?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(Error::PluginFailed(output.status.code()));
        }
        let response: JsonRpcPluginResponse = serde_json::from_slice(&output.stdout)?;
        validate_facts(&response.result.facts)?;
        Ok(response.result.facts)
    }
}

fn validate_facts(facts: &[ExtractedFact]) -> Result<(), Error> {
    for fact in facts {
        if fact.subject.trim().is_empty() {
            return Err(Error::InvalidFact("subject"));
        }
        if fact.predicate.trim().is_empty() {
            return Err(Error::InvalidFact("predicate"));
        }
        if fact.object.trim().is_empty() {
            return Err(Error::InvalidFact("object"));
        }
    }
    Ok(())
}

pub fn extract_facts_for_path(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "rs" => extract_rust_facts(path, source),
        "py" => extract_python_facts(path, source),
        "ts" | "tsx" => extract_typescript_facts(path, source),
        "js" | "jsx" => extract_javascript_facts(path, source),
        "go" => extract_go_facts(path, source),
        "java" => extract_java_facts(path, source),
        "c" | "h" => extract_c_facts(path, source),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => extract_cpp_facts(path, source),
        "cs" => extract_csharp_facts(path, source),
        "rb" => extract_ruby_facts(path, source),
        "php" => extract_php_facts(path, source),
        "kt" | "kts" => extract_kotlin_facts(path, source),
        "swift" => extract_swift_facts(path, source),
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

pub(crate) fn definition_facts(
    path: &str,
    kind: &str,
    names: Vec<String>,
) -> Vec<ExtractedFact> {
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
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid extracted fact field: {0}")]
    InvalidFact(&'static str),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("plugin stdin unavailable")]
    PluginMissingStdin,
    #[error("plugin exited unsuccessfully: {0:?}")]
    PluginFailed(Option<i32>),
}
