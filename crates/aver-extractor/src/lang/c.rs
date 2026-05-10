use crate::{
    Error, ExtractedFact, collect_c_style_function_names, collect_names_from_kinds,
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
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["struct_specifier"])
}

pub fn extract_c_enums(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c::language())?;
    collect_names_from_kinds(tree.root_node(), source.as_bytes(), &["enum_specifier"])
}

pub fn extract_c_type_aliases(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_c::language())?;
    let mut aliases = Vec::new();
    collect_type_definition_aliases(tree.root_node(), source.as_bytes(), &mut aliases)?;
    Ok(aliases)
}

pub fn extract_c_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let mut facts = definition_facts(path, "Function", extract_c_functions(source)?);
    facts.extend(definition_facts(path, "Struct", extract_c_structs(source)?));
    facts.extend(definition_facts(path, "Enum", extract_c_enums(source)?));
    facts.extend(definition_facts(
        path,
        "TypeAlias",
        extract_c_type_aliases(source)?,
    ));
    Ok(facts)
}
