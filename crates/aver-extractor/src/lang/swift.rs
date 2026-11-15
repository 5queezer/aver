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
    let tree = parse_with_language(source, tree_sitter_swift::language())?;
    let root = tree.root_node();
    let source = source.as_bytes();

    let mut facts = definition_facts(
        path,
        "Function",
        collect_names_from_kinds(root, source, &["function_declaration"])?,
    );
    facts.extend(definition_facts(
        path,
        "Class",
        collect_names_from_kinds_with_field_text(
            root,
            source,
            &["class_declaration"],
            "declaration_kind",
            "class",
        )?,
    ));
    facts.extend(definition_facts(
        path,
        "Struct",
        collect_names_from_kinds_with_field_text(
            root,
            source,
            &["class_declaration"],
            "declaration_kind",
            "struct",
        )?,
    ));
    facts.extend(definition_facts(
        path,
        "Enum",
        collect_names_from_kinds_with_field_text(
            root,
            source,
            &["class_declaration"],
            "declaration_kind",
            "enum",
        )?,
    ));
    facts.extend(definition_facts(
        path,
        "Actor",
        collect_names_from_kinds_with_field_text(
            root,
            source,
            &["class_declaration"],
            "declaration_kind",
            "actor",
        )?,
    ));
    let protocols = collect_names_from_kinds(root, source, &["protocol_declaration"])?;
    facts.extend(definition_facts(path, "Protocol", protocols.clone()));

    let protocols = protocols.into_iter().collect::<HashSet<_>>();
    collect_swift_extends_facts(root, source, &protocols, &mut facts)?;
    collect_swift_implements_facts(root, source, &protocols, &mut facts)?;
    Ok(facts)
}

/// Maps a Swift `class_declaration` to its fact kind, keeping actors distinct
/// from classes so one entity never gets two identities.
fn swift_declaration_kind(node: Node<'_>, source: &[u8]) -> Option<&'static str> {
    if node.kind() != "class_declaration" {
        return None;
    }
    match node
        .child_by_field_name("declaration_kind")
        .and_then(|kind| kind.utf8_text(source).ok())
    {
        Some("class") => Some("Class"),
        Some("actor") => Some("Actor"),
        Some("struct") => Some("Struct"),
        Some("enum") => Some("Enum"),
        _ => None,
    }
}

fn collect_swift_implements_facts(
    node: Node<'_>,
    source: &[u8],
    protocols: &HashSet<String>,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if let Some(type_kind) = swift_declaration_kind(node, source)
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

/// Collects the types named by a declaration's own `inheritance_specifier`
/// children, without descending into the body where nested declarations have
/// their own inheritance clauses.
fn collect_swift_inheritance_type_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "inheritance_specifier" {
            continue;
        }
        if let Some(inherits_from) = child.child_by_field_name("inherits_from")
            && let Some(name) = first_named_descendant_of_kind(inherits_from, "type_identifier")
        {
            names.push(name.utf8_text(source)?.to_string());
        }
    }
    Ok(())
}

fn collect_swift_extends_facts(
    node: Node<'_>,
    source: &[u8],
    protocols: &HashSet<String>,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    let class_kind =
        swift_declaration_kind(node, source).filter(|kind| matches!(*kind, "Class" | "Actor"));
    let type_kind = if node.kind() == "protocol_declaration" {
        Some("Protocol")
    } else {
        class_kind
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
        } else if let Some(base_name) = base_names
            .into_iter()
            .find(|base_name| !protocols.contains(base_name))
        {
            // The superclass is the first inheritance entry that is not a
            // file-locally declared protocol.
            facts.push(ExtractedFact {
                subject,
                predicate: "extends".to_string(),
                object: format!("Class:{base_name}"),
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_swift_extends_facts(child, source, protocols, facts)?;
    }
    Ok(())
}
