use tree_sitter::Node;

use crate::{
    Error, ExtractedFact, collect_c_style_function_names, collect_named_nodes,
    collect_names_from_kinds, collect_type_definition_aliases, definition_facts,
    first_named_descendant_of_kind, parse_with_language,
};

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
