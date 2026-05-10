use std::collections::HashSet;

use tree_sitter::Node;

use crate::{
    Error, ExtractedFact, collect_names_from_kinds, definition_facts,
    first_named_descendant_of_kind, parse_with_language,
};

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
