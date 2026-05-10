use std::collections::HashSet;

use tree_sitter::Node;

use crate::{
    Error, ExtractedFact, collect_names_from_kinds, collect_names_from_kinds_with_field_text,
    definition_facts, first_named_descendant_of_kind, parse_with_language,
};

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
