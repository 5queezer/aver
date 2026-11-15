use std::collections::HashSet;

use tree_sitter::Node;

use crate::{
    Error, ExtractedFact, collect_c_style_function_names, collect_named_nodes,
    collect_names_from_kinds, collect_names_from_kinds_requiring_field,
    collect_type_definition_aliases, definition_facts, named_child_of_kind, parse_with_language,
};

pub fn extract_cpp_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_cpp::language())?;
    let mut functions = Vec::new();
    collect_c_style_function_names(tree.root_node(), source.as_bytes(), &mut functions)?;
    Ok(functions)
}

pub fn extract_cpp_classes(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_cpp::language())?;
    collect_names_from_kinds_requiring_field(
        tree.root_node(),
        source.as_bytes(),
        &["class_specifier"],
        "body",
    )
}

pub fn extract_cpp_structs(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_cpp::language())?;
    collect_names_from_kinds_requiring_field(
        tree.root_node(),
        source.as_bytes(),
        &["struct_specifier"],
        "body",
    )
}

pub fn extract_cpp_enums(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_cpp::language())?;
    collect_names_from_kinds_requiring_field(
        tree.root_node(),
        source.as_bytes(),
        &["enum_specifier"],
        "body",
    )
}

pub fn extract_cpp_namespaces(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_cpp::language())?;
    collect_names_from_kinds(
        tree.root_node(),
        source.as_bytes(),
        &["namespace_definition"],
    )
}

pub fn extract_cpp_type_aliases(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_cpp::language())?;
    let mut aliases = Vec::new();
    collect_cpp_type_alias_names(tree.root_node(), source.as_bytes(), &mut aliases)?;
    Ok(aliases)
}

fn collect_cpp_type_alias_names(
    node: Node<'_>,
    source: &[u8],
    aliases: &mut Vec<String>,
) -> Result<(), Error> {
    collect_named_nodes(node, source, &["alias_declaration"], aliases)?;
    collect_type_definition_aliases(node, source, aliases)?;
    Ok(())
}

pub fn extract_cpp_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_cpp::language())?;
    let root = tree.root_node();
    let source = source.as_bytes();

    let mut functions = Vec::new();
    collect_c_style_function_names(root, source, &mut functions)?;
    let mut facts = definition_facts(path, "Function", functions);
    facts.extend(definition_facts(
        path,
        "Class",
        collect_names_from_kinds_requiring_field(root, source, &["class_specifier"], "body")?,
    ));
    facts.extend(definition_facts(
        path,
        "Struct",
        collect_names_from_kinds_requiring_field(root, source, &["struct_specifier"], "body")?,
    ));
    facts.extend(definition_facts(
        path,
        "Enum",
        collect_names_from_kinds_requiring_field(root, source, &["enum_specifier"], "body")?,
    ));
    let mut aliases = Vec::new();
    collect_cpp_type_alias_names(root, source, &mut aliases)?;
    facts.extend(definition_facts(path, "TypeAlias", aliases));
    facts.extend(definition_facts(
        path,
        "Namespace",
        collect_names_from_kinds(root, source, &["namespace_definition"])?,
    ));

    // Kind lookup for base types: forward declarations (`class Bar;`) carry no
    // body but still tell us whether a base is a class or a struct.
    let class_names = collect_names_from_kinds(root, source, &["class_specifier"])?
        .into_iter()
        .collect::<HashSet<_>>();
    let struct_names = collect_names_from_kinds(root, source, &["struct_specifier"])?
        .into_iter()
        .collect::<HashSet<_>>();
    collect_cpp_extends_facts(root, source, &class_names, &struct_names, &mut facts)?;
    Ok(facts)
}

fn collect_cpp_extends_facts(
    node: Node<'_>,
    source: &[u8],
    class_names: &HashSet<String>,
    struct_names: &HashSet<String>,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    let type_kind = match node.kind() {
        "class_specifier" => Some("Class"),
        "struct_specifier" => Some("Struct"),
        _ => None,
    };
    if let Some(type_kind) = type_kind
        && let Some(type_name) = node.child_by_field_name("name")
        && let Some(base_clause) = named_child_of_kind(node, "base_class_clause")
    {
        let mut base_names = Vec::new();
        collect_cpp_base_type_names(base_clause, source, &mut base_names)?;
        let subject = format!("{}:{}", type_kind, type_name.utf8_text(source)?);
        facts.extend(base_names.into_iter().map(|base_name| {
            let base_kind = if class_names.contains(&base_name) {
                "Class"
            } else if struct_names.contains(&base_name) {
                "Struct"
            } else {
                type_kind
            };
            ExtractedFact {
                subject: subject.clone(),
                predicate: "extends".to_string(),
                object: format!("{base_kind}:{base_name}"),
            }
        }));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_cpp_extends_facts(child, source, class_names, struct_names, facts)?;
    }
    Ok(())
}

fn collect_cpp_base_type_names(
    node: Node<'_>,
    source: &[u8],
    names: &mut Vec<String>,
) -> Result<(), Error> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        match child.kind() {
            "type_identifier" | "qualified_identifier" => {
                names.push(child.utf8_text(source)?.to_string());
            }
            "template_type" => {
                if let Some(name) = child.child_by_field_name("name") {
                    names.push(name.utf8_text(source)?.to_string());
                }
            }
            _ => collect_cpp_base_type_names(child, source, names)?,
        }
    }
    Ok(())
}
