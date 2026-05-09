//! Deterministic source extractors (ADR-0007).

pub mod prose;

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use tree_sitter::{Language, Node, Parser};

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

pub fn extract_python_classes(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_python::language())?;
    let mut classes = Vec::new();
    collect_named_nodes(
        tree.root_node(),
        source.as_bytes(),
        &["class_definition"],
        &mut classes,
    )?;
    Ok(classes)
}

pub fn extract_python_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = definition_facts(path, "Function", extract_python_functions(source)?);
    facts.extend(definition_facts(
        path,
        "Class",
        extract_python_classes(source)?,
    ));
    facts.extend(extract_python_extends_facts(source)?);
    Ok(facts)
}

fn extract_python_extends_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_python::language())?;
    let mut facts = Vec::new();
    collect_python_extends_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
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
    collect_function_variable_names(tree.root_node(), source.as_bytes(), &mut functions)?;
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

pub fn extract_typescript_interfaces(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_typescript::language_typescript())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["interface_declaration"],
    )
}

pub fn extract_typescript_type_aliases(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_typescript::language_typescript())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["type_alias_declaration"],
    )
}

pub fn extract_typescript_enums(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_typescript::language_typescript())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["enum_declaration"])
}

pub fn extract_typescript_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = definition_facts(path, "Function", extract_typescript_functions(source)?);
    facts.extend(definition_facts(
        path,
        "Class",
        extract_typescript_classes(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Interface",
        extract_typescript_interfaces(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "TypeAlias",
        extract_typescript_type_aliases(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Enum",
        extract_typescript_enums(source)?,
    ));
    facts.extend(extract_typescript_extends_facts(source)?);
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

pub fn extract_go_structs(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_go::language())?;
    let mut structs = Vec::new();
    collect_go_type_names(
        tree.root_node(),
        source.as_bytes(),
        "struct_type",
        &mut structs,
    )?;
    Ok(structs)
}

pub fn extract_go_interfaces(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_go::language())?;
    let mut interfaces = Vec::new();
    collect_go_type_names(
        tree.root_node(),
        source.as_bytes(),
        "interface_type",
        &mut interfaces,
    )?;
    Ok(interfaces)
}

pub fn extract_go_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = definition_facts(path, "Function", extract_go_functions(source)?);
    facts.extend(definition_facts(
        path,
        "Struct",
        extract_go_structs(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Interface",
        extract_go_interfaces(source)?,
    ));
    facts.extend(extract_go_extends_facts(source)?);
    facts.extend(extract_go_struct_embedding_facts(source)?);
    Ok(facts)
}

fn extract_go_struct_embedding_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let structs = extract_go_structs(source)?
        .into_iter()
        .collect::<HashSet<_>>();
    let tree = parse_with_language(source, tree_sitter_go::language())?;
    let mut facts = Vec::new();
    collect_go_struct_embedding_facts(tree.root_node(), source.as_bytes(), &structs, &mut facts)?;
    Ok(facts)
}

fn extract_go_extends_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_go::language())?;
    let mut facts = Vec::new();
    collect_go_extends_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

pub fn extract_javascript_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_javascript::language())?;
    let mut functions = collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["function_declaration", "method_definition"],
    )?;
    collect_function_variable_names(tree.root_node(), source.as_bytes(), &mut functions)?;
    Ok(functions)
}

pub fn extract_javascript_classes(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_javascript::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["class_declaration"])
}

pub fn extract_javascript_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = definition_facts(path, "Function", extract_javascript_functions(source)?);
    facts.extend(definition_facts(
        path,
        "Class",
        extract_javascript_classes(source)?,
    ));
    facts.extend(extract_javascript_extends_facts(source)?);
    Ok(facts)
}

fn extract_javascript_extends_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_javascript::language())?;
    let mut facts = Vec::new();
    collect_javascript_extends_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

pub fn extract_java_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_java::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["method_declaration"])
}

pub fn extract_java_classes(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_java::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["class_declaration"])
}

pub fn extract_java_interfaces(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_java::language())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["interface_declaration"],
    )
}

pub fn extract_java_enums(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_java::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["enum_declaration"])
}

pub fn extract_java_records(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_java::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["record_declaration"])
}

pub fn extract_java_annotations(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_java::language())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["annotation_type_declaration"],
    )
}

pub fn extract_java_packages(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_java::language())?;
    let mut packages = Vec::new();
    collect_java_package_names(tree.root_node(), source.as_bytes(), &mut packages)?;
    Ok(packages)
}

pub fn extract_java_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = definition_facts(path, "Function", extract_java_functions(source)?);
    facts.extend(definition_facts(
        path,
        "Class",
        extract_java_classes(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Interface",
        extract_java_interfaces(source)?,
    ));
    facts.extend(definition_facts(path, "Enum", extract_java_enums(source)?));
    facts.extend(definition_facts(
        path,
        "Record",
        extract_java_records(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Annotation",
        extract_java_annotations(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Package",
        extract_java_packages(source)?,
    ));
    facts.extend(extract_java_extends_facts(source)?);
    facts.extend(extract_java_implements_facts(source)?);
    facts.extend(extract_java_interface_extends_facts(source)?);
    Ok(facts)
}

fn extract_java_interface_extends_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_java::language())?;
    let mut facts = Vec::new();
    collect_java_interface_extends_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

fn extract_java_implements_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_java::language())?;
    let mut facts = Vec::new();
    collect_java_implements_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

fn extract_java_extends_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_java::language())?;
    let mut facts = Vec::new();
    collect_java_extends_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

pub fn extract_c_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c::language())?;
    let mut functions = Vec::new();
    collect_c_style_function_names(tree.root_node(), source.as_bytes(), &mut functions)?;
    Ok(functions)
}

pub fn extract_c_structs(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["struct_specifier"])
}

pub fn extract_c_enums(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["enum_specifier"])
}

pub fn extract_c_type_aliases(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c::language())?;
    let mut aliases = Vec::new();
    collect_type_definition_aliases(tree.root_node(), source.as_bytes(), &mut aliases)?;
    Ok(aliases)
}

pub fn extract_c_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = definition_facts(path, "Function", extract_c_functions(source)?);
    facts.extend(definition_facts(path, "Struct", extract_c_structs(source)?));
    facts.extend(definition_facts(path, "Enum", extract_c_enums(source)?));
    facts.extend(definition_facts(
        path,
        "TypeAlias",
        extract_c_type_aliases(source)?,
    ));
    Ok(facts)
}

pub fn extract_cpp_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_cpp::language())?;
    let mut functions = Vec::new();
    collect_c_style_function_names(tree.root_node(), source.as_bytes(), &mut functions)?;
    Ok(functions)
}

pub fn extract_cpp_classes(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_cpp::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["class_specifier"])
}

pub fn extract_cpp_structs(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_cpp::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["struct_specifier"])
}

pub fn extract_cpp_enums(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_cpp::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["enum_specifier"])
}

pub fn extract_cpp_namespaces(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_cpp::language())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["namespace_definition"],
    )
}

pub fn extract_cpp_type_aliases(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_cpp::language())?;
    let mut aliases = Vec::new();
    collect_named_nodes(
        tree.root_node(),
        source.as_bytes(),
        &["alias_declaration"],
        &mut aliases,
    )?;
    collect_type_definition_aliases(tree.root_node(), source.as_bytes(), &mut aliases)?;
    Ok(aliases)
}

pub fn extract_cpp_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = definition_facts(path, "Function", extract_cpp_functions(source)?);
    facts.extend(definition_facts(
        path,
        "Class",
        extract_cpp_classes(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Struct",
        extract_cpp_structs(source)?,
    ));
    facts.extend(definition_facts(path, "Enum", extract_cpp_enums(source)?));
    facts.extend(definition_facts(
        path,
        "TypeAlias",
        extract_cpp_type_aliases(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Namespace",
        extract_cpp_namespaces(source)?,
    ));
    facts.extend(extract_cpp_extends_facts(source)?);
    Ok(facts)
}

fn extract_cpp_extends_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_cpp::language())?;
    let mut facts = Vec::new();
    collect_cpp_extends_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

pub fn extract_csharp_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c_sharp::language())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["method_declaration", "local_function_statement"],
    )
}

pub fn extract_csharp_classes(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c_sharp::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["class_declaration"])
}

pub fn extract_csharp_interfaces(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c_sharp::language())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["interface_declaration"],
    )
}

pub fn extract_csharp_structs(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c_sharp::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["struct_declaration"])
}

pub fn extract_csharp_enums(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c_sharp::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["enum_declaration"])
}

pub fn extract_csharp_delegates(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c_sharp::language())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["delegate_declaration"],
    )
}

pub fn extract_csharp_records(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c_sharp::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["record_declaration"])
}

pub fn extract_csharp_namespaces(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c_sharp::language())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["namespace_declaration"],
    )
}

pub fn extract_csharp_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = definition_facts(path, "Function", extract_csharp_functions(source)?);
    facts.extend(definition_facts(
        path,
        "Class",
        extract_csharp_classes(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Interface",
        extract_csharp_interfaces(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Struct",
        extract_csharp_structs(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Enum",
        extract_csharp_enums(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Delegate",
        extract_csharp_delegates(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Record",
        extract_csharp_records(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Namespace",
        extract_csharp_namespaces(source)?,
    ));
    facts.extend(extract_csharp_extends_facts(source)?);
    facts.extend(extract_csharp_implements_facts(source)?);
    Ok(facts)
}

fn extract_csharp_implements_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let interfaces = extract_csharp_interfaces(source)?
        .into_iter()
        .collect::<HashSet<_>>();
    let tree = parse_with_language(source, tree_sitter_c_sharp::language())?;
    let mut facts = Vec::new();
    collect_csharp_implements_facts(tree.root_node(), source.as_bytes(), &interfaces, &mut facts)?;
    Ok(facts)
}

fn extract_csharp_extends_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_c_sharp::language())?;
    let mut facts = Vec::new();
    collect_csharp_extends_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

pub fn extract_ruby_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_ruby::language())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["method", "singleton_method"],
    )
}

pub fn extract_ruby_classes(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_ruby::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["class"])
}

pub fn extract_ruby_modules(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_ruby::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["module"])
}

pub fn extract_ruby_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = definition_facts(path, "Function", extract_ruby_functions(source)?);
    facts.extend(definition_facts(
        path,
        "Class",
        extract_ruby_classes(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Module",
        extract_ruby_modules(source)?,
    ));
    facts.extend(extract_ruby_extends_facts(source)?);
    facts.extend(extract_ruby_implements_facts(source)?);
    Ok(facts)
}

fn extract_ruby_implements_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let modules = extract_ruby_modules(source)?
        .into_iter()
        .collect::<HashSet<_>>();
    let tree = parse_with_language(source, tree_sitter_ruby::language())?;
    let mut facts = Vec::new();
    collect_ruby_implements_facts(tree.root_node(), source.as_bytes(), &modules, &mut facts)?;
    Ok(facts)
}

fn extract_ruby_extends_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_ruby::language())?;
    let mut facts = Vec::new();
    collect_ruby_extends_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

pub fn extract_php_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_php::language_php())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["function_definition", "method_declaration"],
    )
}

pub fn extract_php_classes(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_php::language_php())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["class_declaration"])
}

pub fn extract_php_interfaces(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_php::language_php())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["interface_declaration"],
    )
}

pub fn extract_php_enums(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_php::language_php())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["enum_declaration"])
}

pub fn extract_php_traits(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_php::language_php())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["trait_declaration"])
}

pub fn extract_php_namespaces(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_php::language_php())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["namespace_definition"],
    )
}

pub fn extract_php_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = definition_facts(path, "Function", extract_php_functions(source)?);
    facts.extend(definition_facts(
        path,
        "Class",
        extract_php_classes(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Interface",
        extract_php_interfaces(source)?,
    ));
    facts.extend(definition_facts(path, "Enum", extract_php_enums(source)?));
    facts.extend(definition_facts(path, "Trait", extract_php_traits(source)?));
    facts.extend(definition_facts(
        path,
        "Namespace",
        extract_php_namespaces(source)?,
    ));
    facts.extend(extract_php_extends_facts(source)?);
    facts.extend(extract_php_implements_facts(source)?);
    facts.extend(extract_php_trait_use_facts(source)?);
    Ok(facts)
}

fn extract_php_trait_use_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_php::language_php())?;
    let mut facts = Vec::new();
    collect_php_trait_use_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

fn extract_php_implements_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_php::language_php())?;
    let mut facts = Vec::new();
    collect_php_implements_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

fn extract_php_extends_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_php::language_php())?;
    let mut facts = Vec::new();
    collect_php_extends_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

pub fn extract_kotlin_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_kotlin::language())?;
    collect_descendant_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["function_declaration"],
        "simple_identifier",
    )
}

pub fn extract_kotlin_classes(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_kotlin::language())?;
    collect_kotlin_type_names_by_prefix(tree.root_node(), source.as_bytes(), "class ")
}

pub fn extract_kotlin_interfaces(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_kotlin::language())?;
    collect_kotlin_type_names_by_prefix(tree.root_node(), source.as_bytes(), "interface ")
}

pub fn extract_kotlin_enums(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_kotlin::language())?;
    collect_kotlin_type_names_by_prefix(tree.root_node(), source.as_bytes(), "enum class ")
}

pub fn extract_kotlin_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = definition_facts(path, "Function", extract_kotlin_functions(source)?);
    facts.extend(definition_facts(
        path,
        "Class",
        extract_kotlin_classes(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Interface",
        extract_kotlin_interfaces(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Enum",
        extract_kotlin_enums(source)?,
    ));
    facts.extend(extract_kotlin_extends_facts(source)?);
    facts.extend(extract_kotlin_implements_facts(source)?);
    Ok(facts)
}

fn extract_kotlin_implements_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let interfaces = extract_kotlin_interfaces(source)?
        .into_iter()
        .collect::<HashSet<_>>();
    let tree = parse_with_language(source, tree_sitter_kotlin::language())?;
    let mut facts = Vec::new();
    collect_kotlin_implements_facts(tree.root_node(), source.as_bytes(), &interfaces, &mut facts)?;
    Ok(facts)
}

fn extract_kotlin_extends_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_kotlin::language())?;
    let mut facts = Vec::new();
    collect_kotlin_extends_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

pub fn extract_swift_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_swift::language())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["function_declaration"],
    )
}

pub fn extract_swift_classes(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_swift::language())?;
    collect_names_from_kinds_with_field_text(
        tree.root_node(),
        source.as_bytes(),
        &["class_declaration"],
        "declaration_kind",
        "class",
    )
}

pub fn extract_swift_structs(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_swift::language())?;
    collect_names_from_kinds_with_field_text(
        tree.root_node(),
        source.as_bytes(),
        &["class_declaration"],
        "declaration_kind",
        "struct",
    )
}

pub fn extract_swift_enums(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_swift::language())?;
    collect_names_from_kinds_with_field_text(
        tree.root_node(),
        source.as_bytes(),
        &["class_declaration"],
        "declaration_kind",
        "enum",
    )
}

pub fn extract_swift_actors(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_swift::language())?;
    collect_names_from_kinds_with_field_text(
        tree.root_node(),
        source.as_bytes(),
        &["class_declaration"],
        "declaration_kind",
        "actor",
    )
}

pub fn extract_swift_protocols(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_swift::language())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["protocol_declaration"],
    )
}

pub fn extract_swift_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = definition_facts(path, "Function", extract_swift_functions(source)?);
    facts.extend(definition_facts(
        path,
        "Class",
        extract_swift_classes(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Struct",
        extract_swift_structs(source)?,
    ));
    facts.extend(definition_facts(path, "Enum", extract_swift_enums(source)?));
    facts.extend(definition_facts(
        path,
        "Actor",
        extract_swift_actors(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Protocol",
        extract_swift_protocols(source)?,
    ));
    facts.extend(extract_swift_extends_facts(source)?);
    facts.extend(extract_swift_implements_facts(source)?);
    Ok(facts)
}

fn extract_swift_implements_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let protocols = extract_swift_protocols(source)?
        .into_iter()
        .collect::<HashSet<_>>();
    let tree = parse_with_language(source, tree_sitter_swift::language())?;
    let mut facts = Vec::new();
    collect_swift_implements_facts(tree.root_node(), source.as_bytes(), &protocols, &mut facts)?;
    Ok(facts)
}

fn extract_swift_extends_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_swift::language())?;
    let mut facts = Vec::new();
    collect_swift_extends_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
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

fn collect_names_from_kinds(
    node: Node<'_>,
    source: &[u8],
    kinds: &[&str],
) -> Result<Vec<String>, Error> {
    let mut names = Vec::new();
    collect_named_nodes(node, source, kinds, &mut names)?;
    Ok(names)
}

fn definition_facts(path: &str, kind: &str, names: Vec<String>) -> Vec<ExtractedFact> {
    names
        .into_iter()
        .map(|name| ExtractedFact {
            subject: path.to_string(),
            predicate: "defines".to_string(),
            object: format!("{kind}:{name}"),
        })
        .collect()
}

fn collect_descendant_names_from_kinds(
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

fn collect_names_from_kinds_with_field_text(
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

fn collect_type_definition_aliases(
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

fn collect_swift_implements_facts(
    node: Node<'_>,
    source: &[u8],
    protocols: &HashSet<String>,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "class_declaration"
        && let Some(type_kind) = node
            .child_by_field_name("declaration_kind")
            .and_then(|kind| match kind.utf8_text(source).ok()? {
                "class" | "actor" => Some("Class"),
                "struct" => Some("Struct"),
                "enum" => Some("Enum"),
                _ => None,
            })
        && let Some(type_name) = node.child_by_field_name("name")
    {
        let mut protocol_names = Vec::new();
        collect_swift_inheritance_type_names(node, source, &mut protocol_names)?;
        let subject = format!("{}:{}", type_kind, type_name.utf8_text(source)?);
        facts.extend(
            protocol_names
                .into_iter()
                .filter(|protocol_name| protocols.contains(protocol_name))
                .map(|protocol_name| ExtractedFact {
                    subject: subject.clone(),
                    predicate: "implements".to_string(),
                    object: format!("Protocol:{protocol_name}"),
                }),
        );
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_swift_implements_facts(child, source, protocols, facts)?;
    }
    Ok(())
}

fn collect_swift_inheritance_type_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "inheritance_specifier" {
        if let Some(inherits_from) = node.child_by_field_name("inherits_from")
            && let Some(name) = first_named_descendant_of_kind(inherits_from, "type_identifier")
        {
            names.push(name.utf8_text(source)?.to_string());
        }
        return Ok(());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_swift_inheritance_type_names(child, source, names)?;
    }
    Ok(())
}

fn collect_swift_extends_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    let type_kind = if node.kind() == "class_declaration"
        && node
            .child_by_field_name("declaration_kind")
            .is_some_and(|kind| {
                kind.utf8_text(source)
                    .is_ok_and(|text| matches!(text, "class" | "actor"))
            }) {
        Some("Class")
    } else if node.kind() == "protocol_declaration" {
        Some("Protocol")
    } else {
        None
    };
    if let Some(type_kind) = type_kind
        && let Some(type_name) = node.child_by_field_name("name")
    {
        let mut base_names = Vec::new();
        collect_swift_inheritance_type_names(node, source, &mut base_names)?;
        let subject = format!("{}:{}", type_kind, type_name.utf8_text(source)?);
        if type_kind == "Protocol" {
            facts.extend(base_names.into_iter().map(|base_name| ExtractedFact {
                subject: subject.clone(),
                predicate: "extends".to_string(),
                object: format!("Protocol:{base_name}"),
            }));
        } else if let Some(base_name) = base_names.into_iter().next() {
            facts.push(ExtractedFact {
                subject,
                predicate: "extends".to_string(),
                object: format!("Class:{base_name}"),
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_swift_extends_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_csharp_implements_facts(
    node: Node<'_>,
    source: &[u8],
    interfaces: &HashSet<String>,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    let type_kind = match node.kind() {
        "class_declaration" => Some("Class"),
        "record_declaration" => Some("Record"),
        "struct_declaration" => Some("Struct"),
        _ => None,
    };
    if let Some(type_kind) = type_kind
        && let Some(type_name) = node.child_by_field_name("name")
        && let Some(base_list) = first_named_descendant_of_kind(node, "base_list")
    {
        let mut base_names = Vec::new();
        collect_csharp_base_type_names(base_list, source, &mut base_names)?;
        let subject = format!("{}:{}", type_kind, type_name.utf8_text(source)?);
        facts.extend(
            base_names
                .into_iter()
                .filter(|base| interfaces.contains(base))
                .map(|interface_name| ExtractedFact {
                    subject: subject.clone(),
                    predicate: "implements".to_string(),
                    object: format!("Interface:{interface_name}"),
                }),
        );
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_csharp_implements_facts(child, source, interfaces, facts)?;
    }
    Ok(())
}

fn collect_csharp_base_type_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "generic_name" {
        if let Some(name) = first_named_descendant_of_kind(node, "identifier") {
            names.push(name.utf8_text(source)?.to_string());
        }
        return Ok(());
    }
    if node.kind() == "type_argument_list" {
        return Ok(());
    }
    if node.kind() == "identifier" {
        names.push(node.utf8_text(source)?.to_string());
        return Ok(());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_csharp_base_type_names(child, source, names)?;
    }
    Ok(())
}

fn collect_csharp_extends_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    let type_kind = match node.kind() {
        "class_declaration" => Some("Class"),
        "interface_declaration" => Some("Interface"),
        "record_declaration" => Some("Record"),
        _ => None,
    };
    if let Some(type_kind) = type_kind
        && let Some(type_name) = node.child_by_field_name("name")
        && let Some(base_list) = first_named_descendant_of_kind(node, "base_list")
    {
        let subject = format!("{}:{}", type_kind, type_name.utf8_text(source)?);
        if type_kind == "Interface" {
            let mut base_names = Vec::new();
            collect_csharp_base_type_names(base_list, source, &mut base_names)?;
            facts.extend(base_names.into_iter().map(|base_name| ExtractedFact {
                subject: subject.clone(),
                predicate: "extends".to_string(),
                object: format!("Interface:{base_name}"),
            }));
        } else if let Some(base_name) = first_named_descendant_of_kind(base_list, "identifier") {
            facts.push(ExtractedFact {
                subject,
                predicate: "extends".to_string(),
                object: format!("{}:{}", type_kind, base_name.utf8_text(source)?),
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_csharp_extends_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_cpp_extends_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    let type_kind = match node.kind() {
        "class_specifier" => Some("Class"),
        "struct_specifier" => Some("Struct"),
        _ => None,
    };
    if let Some(type_kind) = type_kind
        && let Some(type_name) = node.child_by_field_name("name")
        && let Some(base_clause) = first_named_descendant_of_kind(node, "base_class_clause")
    {
        let mut base_names = Vec::new();
        collect_cpp_base_type_names(base_clause, source, &mut base_names)?;
        let subject = format!("{}:{}", type_kind, type_name.utf8_text(source)?);
        facts.extend(base_names.into_iter().map(|base_name| ExtractedFact {
            subject: subject.clone(),
            predicate: "extends".to_string(),
            object: format!("{}:{base_name}", type_kind),
        }));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_cpp_extends_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_cpp_base_type_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        match child.kind() {
            "type_identifier" | "qualified_identifier" => {
                names.push(child.utf8_text(source)?.to_string());
            }
            "template_type" => {
                if let Some(name) = child.child_by_field_name("name") {
                    names.push(name.utf8_text(source)?.to_string());
                }
            }
            _ => collect_cpp_base_type_names(child, source, names)?,
        }
    }
    Ok(())
}

fn collect_kotlin_implements_facts(
    node: Node<'_>,
    source: &[u8],
    interfaces: &HashSet<String>,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    let type_kind = match node.kind() {
        "class_declaration"
            if node
                .utf8_text(source)?
                .split_whitespace()
                .any(|word| word == "class") =>
        {
            Some("Class")
        }
        "object_declaration" => Some("Object"),
        _ => None,
    };
    if let Some(type_kind) = type_kind
        && let Some(type_name) = first_named_descendant_of_kind(node, "type_identifier")
    {
        let mut interface_names = Vec::new();
        collect_kotlin_delegation_type_names(node, source, &mut interface_names)?;
        let subject = format!("{}:{}", type_kind, type_name.utf8_text(source)?);
        facts.extend(
            interface_names
                .into_iter()
                .filter(|interface_name| interfaces.contains(interface_name))
                .map(|interface_name| ExtractedFact {
                    subject: subject.clone(),
                    predicate: "implements".to_string(),
                    object: format!("Interface:{interface_name}"),
                }),
        );
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_kotlin_implements_facts(child, source, interfaces, facts)?;
    }
    Ok(())
}

fn collect_kotlin_delegation_type_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "delegation_specifier" {
        if let Some(user_type) = first_named_descendant_of_kind(node, "user_type")
            && let Some(name) = first_named_descendant_of_kind(user_type, "type_identifier")
        {
            names.push(name.utf8_text(source)?.to_string());
        }
        return Ok(());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_kotlin_delegation_type_names(child, source, names)?;
    }
    Ok(())
}

fn collect_kotlin_extends_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "class_declaration" {
        let keywords: Vec<&str> = node.utf8_text(source)?.split_whitespace().collect();
        let kind = if keywords.contains(&"class") {
            Some("Class")
        } else if keywords.contains(&"interface") {
            Some("Interface")
        } else {
            None
        };
        if let Some(kind) = kind
            && let Some(name) = first_named_descendant_of_kind(node, "type_identifier")
        {
            let subject = format!("{kind}:{}", name.utf8_text(source)?);
            if kind == "Interface" {
                let mut base_names = Vec::new();
                collect_kotlin_delegation_type_names(node, source, &mut base_names)?;
                facts.extend(base_names.into_iter().map(|base_name| ExtractedFact {
                    subject: subject.clone(),
                    predicate: "extends".to_string(),
                    object: format!("Interface:{base_name}"),
                }));
            } else if let Some(delegation) =
                first_named_descendant_of_kind(node, "delegation_specifier")
                && let Some(base_name) =
                    first_named_descendant_of_kind(delegation, "type_identifier")
            {
                facts.push(ExtractedFact {
                    subject,
                    predicate: "extends".to_string(),
                    object: format!("Class:{}", base_name.utf8_text(source)?),
                });
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_kotlin_extends_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_ruby_implements_facts(
    node: Node<'_>,
    source: &[u8],
    modules: &HashSet<String>,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "class"
        && let Some(class_name) = node.child_by_field_name("name")
    {
        let mut included = Vec::new();
        collect_ruby_include_names(node, source, &mut included)?;
        facts.extend(
            included
                .into_iter()
                .filter(|name| modules.contains(name))
                .map(|module_name| ExtractedFact {
                    subject: format!("Class:{}", class_name.utf8_text(source).unwrap_or_default()),
                    predicate: "implements".to_string(),
                    object: format!("Module:{module_name}"),
                }),
        );
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ruby_implements_facts(child, source, modules, facts)?;
    }
    Ok(())
}

fn collect_ruby_include_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "call"
        && matches!(
            node.utf8_text(source)?.split_whitespace().next(),
            Some("include" | "prepend" | "extend")
        )
    {
        collect_ruby_mixin_argument_names(node, source, names)?;
        return Ok(());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ruby_include_names(child, source, names)?;
    }
    Ok(())
}

fn collect_ruby_mixin_argument_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "scope_resolution" {
        names.push(node.utf8_text(source)?.to_string());
        return Ok(());
    }
    if node.kind() == "constant" {
        names.push(node.utf8_text(source)?.to_string());
        return Ok(());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ruby_mixin_argument_names(child, source, names)?;
    }
    Ok(())
}

fn collect_ruby_extends_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "class"
        && let Some(class_name) = node.child_by_field_name("name")
        && let Some(superclass) = node.child_by_field_name("superclass")
        && let Some(base_name) = first_named_descendant_of_kind(superclass, "scope_resolution")
            .or_else(|| first_named_descendant_of_kind(superclass, "constant"))
    {
        facts.push(ExtractedFact {
            subject: format!("Class:{}", class_name.utf8_text(source)?),
            predicate: "extends".to_string(),
            object: format!("Class:{}", base_name.utf8_text(source)?),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ruby_extends_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_php_trait_use_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "class_declaration"
        && let Some(class_name) = node.child_by_field_name("name")
        && let Some(body) = node.child_by_field_name("body")
    {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            if child.kind() == "use_declaration" {
                let mut trait_names = Vec::new();
                collect_descendant_texts(
                    child,
                    source,
                    &["name", "qualified_name"],
                    &mut trait_names,
                )?;
                let subject = format!("Class:{}", class_name.utf8_text(source)?);
                facts.extend(trait_names.into_iter().map(|trait_name| ExtractedFact {
                    subject: subject.clone(),
                    predicate: "uses".to_string(),
                    object: format!("Trait:{trait_name}"),
                }));
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_php_trait_use_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_php_implements_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "class_declaration"
        && let Some(class_name) = node.child_by_field_name("name")
        && let Some(interfaces) = first_named_descendant_of_kind(node, "class_interface_clause")
    {
        let mut interface_names = Vec::new();
        collect_php_reference_names(interfaces, source, &mut interface_names)?;
        facts.extend(
            interface_names
                .into_iter()
                .map(|interface_name| ExtractedFact {
                    subject: format!("Class:{}", class_name.utf8_text(source).unwrap_or_default()),
                    predicate: "implements".to_string(),
                    object: format!("Interface:{interface_name}"),
                }),
        );
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_php_implements_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_php_reference_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "qualified_name" {
        names.push(node.utf8_text(source)?.to_string());
        return Ok(());
    }
    if node.kind() == "name" {
        names.push(node.utf8_text(source)?.to_string());
        return Ok(());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_php_reference_names(child, source, names)?;
    }
    Ok(())
}

fn collect_php_extends_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "class_declaration"
        && let Some(class_name) = node.child_by_field_name("name")
        && let Some(base_clause) = first_named_descendant_of_kind(node, "base_clause")
        && let Some(base_name) = first_named_descendant_of_kind(base_clause, "qualified_name")
            .or_else(|| first_named_descendant_of_kind(base_clause, "name"))
    {
        facts.push(ExtractedFact {
            subject: format!("Class:{}", class_name.utf8_text(source)?),
            predicate: "extends".to_string(),
            object: format!("Class:{}", base_name.utf8_text(source)?),
        });
    }

    if node.kind() == "interface_declaration"
        && let Some(interface_name) = node.child_by_field_name("name")
        && let Some(base_clause) = first_named_descendant_of_kind(node, "base_clause")
    {
        let mut base_names = Vec::new();
        collect_descendant_texts(
            base_clause,
            source,
            &["name", "qualified_name"],
            &mut base_names,
        )?;
        let subject = format!("Interface:{}", interface_name.utf8_text(source)?);
        facts.extend(base_names.into_iter().map(|base_name| ExtractedFact {
            subject: subject.clone(),
            predicate: "extends".to_string(),
            object: format!("Interface:{base_name}"),
        }));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_php_extends_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_javascript_extends_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "class_declaration"
        && let Some(class_name) = node.child_by_field_name("name")
        && let Some(heritage) = first_named_descendant_of_kind(node, "class_heritage")
        && let Some(base_name) = first_named_descendant_of_kind(heritage, "identifier")
    {
        facts.push(ExtractedFact {
            subject: format!("Class:{}", class_name.utf8_text(source)?),
            predicate: "extends".to_string(),
            object: format!("Class:{}", base_name.utf8_text(source)?),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_javascript_extends_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_java_interface_extends_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "interface_declaration"
        && let Some(interface_name) = node.child_by_field_name("name")
        && let Some(extends_interfaces) = first_named_descendant_of_kind(node, "extends_interfaces")
    {
        let mut base_names = Vec::new();
        collect_java_implements_names(extends_interfaces, source, &mut base_names)?;
        let subject = format!("Interface:{}", interface_name.utf8_text(source)?);
        facts.extend(base_names.into_iter().map(|base_name| ExtractedFact {
            subject: subject.clone(),
            predicate: "extends".to_string(),
            object: format!("Interface:{base_name}"),
        }));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_java_interface_extends_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_java_implements_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    let type_kind = match node.kind() {
        "class_declaration" => Some("Class"),
        "record_declaration" => Some("Record"),
        _ => None,
    };
    if let Some(type_kind) = type_kind
        && let Some(type_name) = node.child_by_field_name("name")
        && let Some(interfaces) = node.child_by_field_name("interfaces")
    {
        let mut interface_names = Vec::new();
        collect_java_implements_names(interfaces, source, &mut interface_names)?;
        let subject = format!("{}:{}", type_kind, type_name.utf8_text(source)?);
        facts.extend(
            interface_names
                .into_iter()
                .map(|interface_name| ExtractedFact {
                    subject: subject.clone(),
                    predicate: "implements".to_string(),
                    object: format!("Interface:{interface_name}"),
                }),
        );
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_java_implements_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_java_extends_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "class_declaration"
        && let Some(class_name) = node.child_by_field_name("name")
        && let Some(superclass) = node.child_by_field_name("superclass")
        && let Some(base_name) = first_named_descendant_of_kind(superclass, "type_identifier")
            .or_else(|| first_named_descendant_of_kind(superclass, "identifier"))
    {
        facts.push(ExtractedFact {
            subject: format!("Class:{}", class_name.utf8_text(source)?),
            predicate: "extends".to_string(),
            object: format!("Class:{}", base_name.utf8_text(source)?),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_java_extends_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_python_extends_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "class_definition"
        && let Some(class_name) = node.child_by_field_name("name")
        && let Some(superclasses) = node.child_by_field_name("superclasses")
    {
        let mut base_names = Vec::new();
        collect_python_superclass_names(superclasses, source, &mut base_names)?;
        let subject = format!("Class:{}", class_name.utf8_text(source)?);
        facts.extend(base_names.into_iter().map(|base_name| ExtractedFact {
            subject: subject.clone(),
            predicate: "extends".to_string(),
            object: format!("Class:{base_name}"),
        }));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_python_extends_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_python_superclass_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        match child.kind() {
            "identifier" | "attribute" => names.push(child.utf8_text(source)?.to_string()),
            "subscript" => {
                if let Some(value) = child.child_by_field_name("value") {
                    names.push(value.utf8_text(source)?.to_string());
                }
            }
            _ => collect_python_superclass_names(child, source, names)?,
        }
    }
    Ok(())
}

fn collect_java_package_names(
    node: Node<'_>,
    source: &[u8],
    packages: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "package_declaration"
        && let Some(name) = first_named_descendant_of_kind(node, "scoped_identifier")
            .or_else(|| first_named_descendant_of_kind(node, "identifier"))
    {
        packages.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_java_package_names(child, source, packages)?;
    }
    Ok(())
}

fn collect_kotlin_type_names_by_prefix(
    node: Node<'_>,
    source: &[u8],
    prefix: &str,
) -> Result<Vec<String>, Error> {
    let mut names = Vec::new();
    collect_kotlin_prefixed_type_names(node, source, prefix, &mut names)?;
    Ok(names)
}

fn collect_kotlin_prefixed_type_names(
    node: Node<'_>,
    source: &[u8],
    prefix: &str,
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "class_declaration"
        && node.utf8_text(source)?.trim_start().starts_with(prefix)
        && let Some(name) = first_named_descendant_of_kind(node, "type_identifier")
    {
        names.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_kotlin_prefixed_type_names(child, source, prefix, names)?;
    }
    Ok(())
}

fn collect_go_struct_embedding_facts(
    node: Node<'_>,
    source: &[u8],
    structs: &HashSet<String>,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "type_spec"
        && let Some(struct_node) = node.child_by_field_name("type")
        && struct_node.kind() == "struct_type"
        && let Some(struct_name) = node.child_by_field_name("name")
    {
        let struct_name = struct_name.utf8_text(source)?;
        let mut embedded = Vec::new();
        collect_go_embedded_struct_names(struct_node, source, &mut embedded)?;
        facts.extend(
            embedded
                .into_iter()
                .filter(|base| base != struct_name && structs.contains(base))
                .map(|base| ExtractedFact {
                    subject: format!("Struct:{struct_name}"),
                    predicate: "extends".to_string(),
                    object: format!("Struct:{base}"),
                }),
        );
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_go_struct_embedding_facts(child, source, structs, facts)?;
    }
    Ok(())
}

fn collect_go_extends_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "type_spec"
        && let Some(interface_node) = node.child_by_field_name("type")
        && interface_node.kind() == "interface_type"
        && let Some(interface_name) = node.child_by_field_name("name")
    {
        let mut embedded = Vec::new();
        collect_go_interface_embedding_names(interface_node, source, &mut embedded)?;
        facts.extend(embedded.into_iter().map(|base| ExtractedFact {
            subject: format!(
                "Interface:{}",
                interface_name.utf8_text(source).unwrap_or_default()
            ),
            predicate: "extends".to_string(),
            object: format!("Interface:{base}"),
        }));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_go_extends_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_go_interface_embedding_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "type_elem" {
        collect_descendant_texts(node, source, &["type_identifier", "qualified_type"], names)?;
        return Ok(());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_go_interface_embedding_names(child, source, names)?;
    }
    Ok(())
}

fn collect_go_embedded_struct_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "field_declaration"
        && first_named_descendant_of_kind(node, "field_identifier").is_none()
        && let Some(name) = first_named_descendant_of_kind(node, "type_identifier")
    {
        names.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_go_embedded_struct_names(child, source, names)?;
    }
    Ok(())
}

fn collect_go_type_names(
    node: Node<'_>,
    source: &[u8],
    type_kind: &str,
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "type_spec"
        && node
            .child_by_field_name("type")
            .is_some_and(|type_node| type_node.kind() == type_kind)
        && let Some(name) = node.child_by_field_name("name")
    {
        names.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_go_type_names(child, source, type_kind, names)?;
    }
    Ok(())
}

fn collect_function_variable_names(
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

fn collect_c_style_function_names(
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

fn collect_descendant_texts(
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

fn first_named_descendant_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
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

fn collect_java_implements_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "generic_type" {
        if let Some(name) = first_named_descendant_of_kind(node, "type_identifier")
            .or_else(|| first_named_descendant_of_kind(node, "scoped_type_identifier"))
        {
            names.push(name.utf8_text(source)?.to_string());
        }
        return Ok(());
    }
    if node.kind() == "type_arguments" {
        return Ok(());
    }
    if matches!(node.kind(), "type_identifier" | "identifier") {
        names.push(node.utf8_text(source)?.to_string());
        return Ok(());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_java_implements_names(child, source, names)?;
    }
    Ok(())
}

fn extract_typescript_extends_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_typescript::language_typescript())?;
    let mut facts = Vec::new();
    collect_typescript_extends_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

fn collect_typescript_extends_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "class_declaration"
        && let Some(class_name) = node.child_by_field_name("name")
        && let Some(heritage) = first_named_descendant_of_kind(node, "class_heritage")
    {
        if let Some(extends_clause) = first_named_descendant_of_kind(heritage, "extends_clause")
            && let Some(base_name) = first_named_descendant_of_kind(extends_clause, "identifier")
                .or_else(|| first_named_descendant_of_kind(extends_clause, "type_identifier"))
        {
            facts.push(ExtractedFact {
                subject: format!("Class:{}", class_name.utf8_text(source)?),
                predicate: "extends".to_string(),
                object: format!("Class:{}", base_name.utf8_text(source)?),
            });
        }

        if let Some(implements_clause) =
            first_named_descendant_of_kind(heritage, "implements_clause")
        {
            let mut interface_names = Vec::new();
            collect_typescript_implements_names(implements_clause, source, &mut interface_names)?;
            let subject = format!("Class:{}", class_name.utf8_text(source)?);
            facts.extend(
                interface_names
                    .into_iter()
                    .map(|interface_name| ExtractedFact {
                        subject: subject.clone(),
                        predicate: "implements".to_string(),
                        object: format!("Interface:{interface_name}"),
                    }),
            );
        }
    }

    if node.kind() == "interface_declaration"
        && let Some(interface_name) = node.child_by_field_name("name")
        && let Some(extends_clause) = first_named_descendant_of_kind(node, "extends_type_clause")
    {
        let mut base_names = Vec::new();
        collect_typescript_implements_names(extends_clause, source, &mut base_names)?;
        let subject = format!("Interface:{}", interface_name.utf8_text(source)?);
        facts.extend(base_names.into_iter().map(|base_name| ExtractedFact {
            subject: subject.clone(),
            predicate: "extends".to_string(),
            object: format!("Interface:{base_name}"),
        }));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_typescript_extends_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_typescript_implements_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        match child.kind() {
            "type_identifier" | "nested_type_identifier" | "identifier" => {
                names.push(child.utf8_text(source)?.to_string());
            }
            "generic_type" => {
                if let Some(name) = child.child_by_field_name("name") {
                    names.push(name.utf8_text(source)?.to_string());
                }
            }
            _ => collect_typescript_implements_names(child, source, names)?,
        }
    }
    Ok(())
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
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("plugin stdin unavailable")]
    PluginMissingStdin,
    #[error("plugin exited unsuccessfully: {0:?}")]
    PluginFailed(Option<i32>),
}
