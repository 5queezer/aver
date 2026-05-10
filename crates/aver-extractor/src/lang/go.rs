use std::collections::HashSet;

use tree_sitter::Node;

use crate::{
    Error, ExtractedFact, collect_descendant_texts, collect_named_nodes, definition_facts,
    first_named_descendant_of_kind, parse_with_language,
};

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
