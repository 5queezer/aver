use tree_sitter::Node;

use crate::{
    Error, ExtractedFact, collect_names_from_kinds, definition_facts,
    first_named_descendant_of_kind, parse_with_language,
};

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
                collect_php_reference_names(child, source, &mut trait_names)?;
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
        collect_php_reference_names(base_clause, source, &mut base_names)?;
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
