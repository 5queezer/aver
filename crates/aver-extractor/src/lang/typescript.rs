use tree_sitter::Node;

use crate::{
    Error, ExtractedFact, collect_function_variable_names, collect_heritage_type_name,
    collect_named_nodes, collect_names_from_kinds, definition_facts, first_named_descendant_of_kind,
    parse_with_language,
};

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
        if let Some(extends_clause) = first_named_descendant_of_kind(heritage, "extends_clause") {
            let mut base_name = None;
            collect_heritage_type_name(extends_clause, source, &mut base_name)?;
            if let Some(base_name) = base_name {
                facts.push(ExtractedFact {
                    subject: format!("Class:{}", class_name.utf8_text(source)?),
                    predicate: "extends".to_string(),
                    object: format!("Class:{base_name}"),
                });
            }
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
