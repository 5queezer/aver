use std::collections::HashSet;

use tree_sitter::Node;

use crate::{
    Error, ExtractedFact, collect_names_from_kinds, definition_facts,
    first_named_descendant_of_kind, parse_with_language,
};

pub fn extract_ruby_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_ruby::language())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["method", "singleton_method"],
    )
}

pub fn extract_ruby_classes(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_ruby::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["class"])
}

pub fn extract_ruby_modules(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_ruby::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["module"])
}

pub fn extract_ruby_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_ruby::language())?;
    let root = tree.root_node();
    let source = source.as_bytes();

    let mut facts = definition_facts(
        path,
        "Function",
        collect_names_from_kinds(root, source, &["method", "singleton_method"])?,
    );
    facts.extend(definition_facts(
        path,
        "Class",
        collect_names_from_kinds(root, source, &["class"])?,
    ));
    let modules = collect_names_from_kinds(root, source, &["module"])?;
    facts.extend(definition_facts(path, "Module", modules.clone()));

    collect_ruby_extends_facts(root, source, &mut facts)?;
    let modules = modules.into_iter().collect::<HashSet<_>>();
    collect_ruby_implements_facts(root, source, &modules, &mut facts)?;
    Ok(facts)
}

fn collect_ruby_implements_facts(
    node: Node<'_>,
    source: &[u8],
    modules: &HashSet<String>,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "class"
        && let Some(class_name) = node.child_by_field_name("name")
    {
        let mut included = Vec::new();
        collect_ruby_include_names(node, source, &mut included)?;
        let class_name = class_name.utf8_text(source)?;
        facts.extend(
            included
                .into_iter()
                .filter(|name| modules.contains(name))
                .map(|module_name| ExtractedFact {
                    subject: format!("Class:{class_name}"),
                    predicate: "implements".to_string(),
                    object: format!("Module:{module_name}"),
                }),
        );
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ruby_implements_facts(child, source, modules, facts)?;
    }
    Ok(())
}

fn collect_ruby_include_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "call"
        && matches!(
            node.utf8_text(source)?.split_whitespace().next(),
            Some("include" | "prepend" | "extend")
        )
    {
        collect_ruby_mixin_argument_names(node, source, names)?;
        return Ok(());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ruby_include_names(child, source, names)?;
    }
    Ok(())
}

fn collect_ruby_mixin_argument_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "scope_resolution" {
        names.push(node.utf8_text(source)?.to_string());
        return Ok(());
    }
    if node.kind() == "constant" {
        names.push(node.utf8_text(source)?.to_string());
        return Ok(());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ruby_mixin_argument_names(child, source, names)?;
    }
    Ok(())
}

fn collect_ruby_extends_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "class"
        && let Some(class_name) = node.child_by_field_name("name")
        && let Some(superclass) = node.child_by_field_name("superclass")
        && let Some(base_name) = first_named_descendant_of_kind(superclass, "scope_resolution")
            .or_else(|| first_named_descendant_of_kind(superclass, "constant"))
    {
        facts.push(ExtractedFact {
            subject: format!("Class:{}", class_name.utf8_text(source)?),
            predicate: "extends".to_string(),
            object: format!("Class:{}", base_name.utf8_text(source)?),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ruby_extends_facts(child, source, facts)?;
    }
    Ok(())
}
