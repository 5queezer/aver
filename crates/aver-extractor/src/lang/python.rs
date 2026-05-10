use tree_sitter::Node;

use crate::{Error, ExtractedFact, collect_named_nodes, definition_facts, parse_with_language};

pub fn extract_python_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_python::language())?;
    let mut functions = Vec::new();
    collect_named_nodes(
        tree.root_node(),
        source.as_bytes(),
        &["function_definition"],
        &mut functions,
    )?;
    Ok(functions)
}

pub fn extract_python_classes(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_python::language())?;
    let mut classes = Vec::new();
    collect_named_nodes(
        tree.root_node(),
        source.as_bytes(),
        &["class_definition"],
        &mut classes,
    )?;
    Ok(classes)
}

pub fn extract_python_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = definition_facts(path, "Function", extract_python_functions(source)?);
    facts.extend(definition_facts(
        path,
        "Class",
        extract_python_classes(source)?,
    ));
    facts.extend(extract_python_extends_facts(source)?);
    Ok(facts)
}

fn extract_python_extends_facts(source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_python::language())?;
    let mut facts = Vec::new();
    collect_python_extends_facts(tree.root_node(), source.as_bytes(), &mut facts)?;
    Ok(facts)
}

fn collect_python_extends_facts(
    node: Node<'_>,
    source: &[u8],
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    if node.kind() == "class_definition"
        && let Some(class_name) = node.child_by_field_name("name")
        && let Some(superclasses) = node.child_by_field_name("superclasses")
    {
        let mut base_names = Vec::new();
        collect_python_superclass_names(superclasses, source, &mut base_names)?;
        let subject = format!("Class:{}", class_name.utf8_text(source)?);
        facts.extend(base_names.into_iter().map(|base_name| ExtractedFact {
            subject: subject.clone(),
            predicate: "extends".to_string(),
            object: format!("Class:{base_name}"),
        }));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_python_extends_facts(child, source, facts)?;
    }
    Ok(())
}

fn collect_python_superclass_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        match child.kind() {
            "identifier" | "attribute" => names.push(child.utf8_text(source)?.to_string()),
            "subscript" => {
                if let Some(value) = child.child_by_field_name("value") {
                    names.push(value.utf8_text(source)?.to_string());
                }
            }
            _ => collect_python_superclass_names(child, source, names)?,
        }
    }
    Ok(())
}
