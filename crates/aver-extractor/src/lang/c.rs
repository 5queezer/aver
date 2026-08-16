use crate::{
    Error, ExtractedFact, collect_c_style_function_names, collect_names_from_kinds_requiring_field,
    collect_type_definition_aliases, definition_facts, parse_with_language,
};

pub fn extract_c_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c::language())?;
    let mut functions = Vec::new();
    collect_c_style_function_names(tree.root_node(), source.as_bytes(), &mut functions)?;
    Ok(functions)
}

pub fn extract_c_structs(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c::language())?;
    collect_names_from_kinds_requiring_field(
        tree.root_node(),
        source.as_bytes(),
        &["struct_specifier"],
        "body",
    )
}

pub fn extract_c_enums(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c::language())?;
    collect_names_from_kinds_requiring_field(
        tree.root_node(),
        source.as_bytes(),
        &["enum_specifier"],
        "body",
    )
}

pub fn extract_c_type_aliases(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c::language())?;
    let mut aliases = Vec::new();
    collect_type_definition_aliases(tree.root_node(), source.as_bytes(), &mut aliases)?;
    Ok(aliases)
}

pub fn extract_c_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_c::language())?;
    let root = tree.root_node();
    let source = source.as_bytes();

    let mut functions = Vec::new();
    collect_c_style_function_names(root, source, &mut functions)?;
    let mut facts = definition_facts(path, "Function", functions);
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
    collect_type_definition_aliases(root, source, &mut aliases)?;
    facts.extend(definition_facts(path, "TypeAlias", aliases));
    Ok(facts)
}
