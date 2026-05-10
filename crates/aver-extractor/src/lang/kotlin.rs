use std::collections::HashSet;

use tree_sitter::Node;

use crate::{
    Error, ExtractedFact, collect_descendant_names_from_kinds, definition_facts,
    first_named_descendant_of_kind, parse_with_language,
};

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
