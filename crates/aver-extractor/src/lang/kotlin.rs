use std::collections::HashSet;

use tree_sitter::Node;

use crate::{
    Error, ExtractedFact, collect_descendant_names_from_kinds, definition_facts,
    first_named_descendant_of_kind, named_child_of_kind, parse_with_language,
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
    let mut names = Vec::new();
    collect_kotlin_type_names(tree.root_node(), source.as_bytes(), "Class", &mut names)?;
    Ok(names)
}

pub fn extract_kotlin_interfaces(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_kotlin::language())?;
    let mut names = Vec::new();
    collect_kotlin_type_names(tree.root_node(), source.as_bytes(), "Interface", &mut names)?;
    Ok(names)
}

pub fn extract_kotlin_enums(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_kotlin::language())?;
    let mut names = Vec::new();
    collect_kotlin_type_names(tree.root_node(), source.as_bytes(), "Enum", &mut names)?;
    Ok(names)
}

pub fn extract_kotlin_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_kotlin::language())?;
    let root = tree.root_node();
    let source = source.as_bytes();

    let mut facts = definition_facts(
        path,
        "Function",
        collect_descendant_names_from_kinds(
            root,
            source,
            &["function_declaration"],
            "simple_identifier",
        )?,
    );

    let mut classes = Vec::new();
    collect_kotlin_type_names(root, source, "Class", &mut classes)?;
    facts.extend(definition_facts(path, "Class", classes));

    let mut interfaces = Vec::new();
    collect_kotlin_type_names(root, source, "Interface", &mut interfaces)?;
    facts.extend(definition_facts(path, "Interface", interfaces.clone()));

    let mut enums = Vec::new();
    collect_kotlin_type_names(root, source, "Enum", &mut enums)?;
    facts.extend(definition_facts(path, "Enum", enums));

    let interfaces = interfaces.into_iter().collect::<HashSet<_>>();
    collect_kotlin_extends_facts(root, source, &interfaces, &mut facts)?;
    collect_kotlin_implements_facts(root, source, &interfaces, &mut facts)?;
    Ok(facts)
}

/// Classifies a Kotlin `class_declaration` by inspecting its own keyword
/// tokens (`class` / `interface` / `enum`), so modifiers (`data`, `sealed`),
/// annotations, and comments mentioning "class" cannot confuse the result.
fn kotlin_declaration_kind(node: Node<'_>) -> Option<&'static str> {
    if node.kind() != "class_declaration" {
        return None;
    }
    let mut saw_enum_keyword = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            continue;
        }
        match child.kind() {
            "interface" => return Some("Interface"),
            "enum" => saw_enum_keyword = true,
            "class" => return Some(if saw_enum_keyword { "Enum" } else { "Class" }),
            _ => {}
        }
    }
    None
}

fn collect_kotlin_type_names(
    node: Node<'_>,
    source: &[u8],
    wanted_kind: &str,
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if kotlin_declaration_kind(node) == Some(wanted_kind)
        && let Some(name) = named_child_of_kind(node, "type_identifier")
    {
        names.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_kotlin_type_names(child, source, wanted_kind, names)?;
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
        "class_declaration" => kotlin_declaration_kind(node).filter(|kind| *kind != "Interface"),
        "object_declaration" => Some("Object"),
        _ => None,
    };
    if let Some(type_kind) = type_kind
        && let Some(type_name) = named_child_of_kind(node, "type_identifier")
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

/// Collects the types named by a declaration's own `delegation_specifier`
/// children (`class Store : BaseStore(), Recallable`), without descending
/// into the body where nested declarations have their own specifiers.
fn collect_kotlin_delegation_type_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "delegation_specifier" {
            continue;
        }
        if let Some(user_type) = first_named_descendant_of_kind(child, "user_type")
            && let Some(name) = first_named_descendant_of_kind(user_type, "type_identifier")
        {
            names.push(name.utf8_text(source)?.to_string());
        }
    }
    Ok(())
}

fn collect_kotlin_extends_facts(
    node: Node<'_>,
    source: &[u8],
    interfaces: &HashSet<String>,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if let Some(kind) = kotlin_declaration_kind(node)
        && let Some(name) = named_child_of_kind(node, "type_identifier")
    {
        let subject = format!("{kind}:{}", name.utf8_text(source)?);
        let mut base_names = Vec::new();
        collect_kotlin_delegation_type_names(node, source, &mut base_names)?;
        if kind == "Interface" {
            facts.extend(base_names.into_iter().map(|base_name| ExtractedFact {
                subject: subject.clone(),
                predicate: "extends".to_string(),
                object: format!("Interface:{base_name}"),
            }));
        } else if kind == "Class"
            && let Some(base_name) = base_names
                .into_iter()
                .find(|base_name| !interfaces.contains(base_name))
        {
            // The superclass is the first delegation entry that is not a
            // file-locally declared interface.
            facts.push(ExtractedFact {
                subject,
                predicate: "extends".to_string(),
                object: format!("Class:{base_name}"),
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_kotlin_extends_facts(child, source, interfaces, facts)?;
    }
    Ok(())
}
