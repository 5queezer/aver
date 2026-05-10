use tree_sitter::Node;

use crate::{
    Error, ExtractedFact, collect_names_from_kinds, definition_facts,
    first_named_descendant_of_kind, parse_with_language,
};

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
