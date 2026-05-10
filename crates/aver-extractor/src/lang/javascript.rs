use tree_sitter::Node;

use crate::{
    Error, ExtractedFact, collect_function_variable_names, collect_heritage_type_name,
    collect_names_from_kinds, definition_facts, first_named_descendant_of_kind,
    parse_with_language,
};

pub fn extract_javascript_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_javascript::language())?;
    let mut functions = collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["function_declaration", "method_definition"],
    )?;
    collect_function_variable_names(tree.root_node(), source.as_bytes(), &mut functions)?;
    Ok(functions)
}

pub fn extract_javascript_classes(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_javascript::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["class_declaration"])
}

pub fn extract_javascript_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = definition_facts(path, "Function", extract_javascript_functions(source)?);
    facts.extend(definition_facts(
        path,
        "Class",
        extract_javascript_classes(source)?,
    ));
    facts.extend(extract_javascript_extends_facts(source)?);
    Ok(facts)
}

fn extract_javascript_extends_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_javascript::language())?;
    let mut facts = Vec::new();
    collect_javascript_extends_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

fn collect_javascript_extends_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "class_declaration"
        && let Some(class_name) = node.child_by_field_name("name")
        && let Some(heritage) = first_named_descendant_of_kind(node, "class_heritage")
    {
        let mut base_name = None;
        collect_heritage_type_name(heritage, source, &mut base_name)?;
        if let Some(base_name) = base_name {
            facts.push(ExtractedFact {
                subject: format!("Class:{}", class_name.utf8_text(source)?),
                predicate: "extends".to_string(),
                object: format!("Class:{base_name}"),
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_javascript_extends_facts(child, source, facts)?;
    }
    Ok(())
}
