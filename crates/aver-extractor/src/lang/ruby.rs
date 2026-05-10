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
    let mut facts = definition_facts(path, "Function", extract_ruby_functions(source)?);
    facts.extend(definition_facts(
        path,
        "Class",
        extract_ruby_classes(source)?,
    ));
    facts.extend(definition_facts(
        path,
        "Module",
        extract_ruby_modules(source)?,
    ));
    facts.extend(extract_ruby_extends_facts(source)?);
    facts.extend(extract_ruby_implements_facts(source)?);
    Ok(facts)
}

fn extract_ruby_implements_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let modules = extract_ruby_modules(source)?
        .into_iter()
        .collect::<HashSet<_>>();
    let tree = parse_with_language(source, tree_sitter_ruby::language())?;
    let mut facts = Vec::new();
    collect_ruby_implements_facts(tree.root_node(), source.as_bytes(), &modules, &mut facts)?;
    Ok(facts)
}

fn extract_ruby_extends_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_ruby::language())?;
    let mut facts = Vec::new();
    collect_ruby_extends_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
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
        facts.extend(
            included
                .into_iter()
                .filter(|name| modules.contains(name))
                .map(|module_name| ExtractedFact {
                    subject: format!("Class:{}", class_name.utf8_text(source).unwrap_or_default()),
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
