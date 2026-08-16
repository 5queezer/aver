use std::collections::HashSet;

use tree_sitter::Node;

use crate::{Error, ExtractedFact, parse_with_language};

pub fn extract_rust_functions(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_rust::language())?;

    let mut functions = Vec::new();
    collect_function_names(tree.root_node(), source.as_bytes(), &mut functions)?;
    Ok(functions)
}

pub fn extract_rust_imports(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_rust::language())?;

    let mut imports = Vec::new();
    collect_imports(tree.root_node(), source.as_bytes(), &mut imports)?;

    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for import in imports {
        if seen.insert(import.clone()) {
            deduped.push(import);
        }
    }

    Ok(deduped)
}

pub fn extract_rust_calls(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_rust::language())?;

    let mut calls = Vec::new();
    collect_calls(tree.root_node(), source.as_bytes(), &mut calls)?;
    Ok(calls)
}

pub fn extract_rust_structs(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_rust::language())?;

    let mut structs = Vec::new();
    collect_structs(tree.root_node(), source.as_bytes(), &mut structs)?;
    Ok(structs)
}

pub fn extract_rust_enums(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_rust::language())?;

    let mut enums = Vec::new();
    collect_enums(tree.root_node(), source.as_bytes(), &mut enums)?;
    Ok(enums)
}

pub fn extract_rust_traits(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_rust::language())?;

    let mut traits = Vec::new();
    collect_traits(tree.root_node(), source.as_bytes(), &mut traits)?;
    Ok(traits)
}

pub fn extract_rust_consts(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_rust::language())?;

    let mut consts = Vec::new();
    collect_consts(tree.root_node(), source.as_bytes(), &mut consts)?;
    Ok(consts)
}

pub fn extract_rust_modules(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_rust::language())?;

    let mut modules = Vec::new();
    collect_modules(tree.root_node(), source.as_bytes(), &mut modules)?;
    Ok(modules)
}

pub fn extract_rust_tests(source: &str) -> Result<Vec<String>, Error> {
    let tree = parse_with_language(source, tree_sitter_rust::language())?;

    let mut tests = Vec::new();
    collect_tests(tree.root_node(), source.as_bytes(), &mut tests)?;
    Ok(tests)
}

pub fn map_rust_tests_to_functions(source: &str) -> Result<Vec<(String, String)>, Error> {
    let tests = extract_rust_tests(source)?;
    let test_names = tests.iter().cloned().collect::<HashSet<_>>();
    let functions = extract_rust_functions(source)?
        .into_iter()
        .filter(|function| !test_names.contains(function))
        .collect::<Vec<_>>();

    Ok(map_tests_to_functions(&tests, &functions))
}

/// Extracts all facts from a single parse of the file.
///
/// Every entity gets ONE canonical identity: top-level items are named
/// (`Function:foo`, `Struct:Bar`), items inside modules are module-qualified
/// (`Function:m::foo`), and impl methods are type-qualified
/// (`Function:Type::method`). Call edges use the caller's qualified identity
/// and qualify bare callees with the caller's module path.
pub fn extract_rust_facts(path: &str, source: &str) -> Result<Vec<ExtractedFact>, Error> {
    let tree = parse_with_language(source, tree_sitter_rust::language())?;

    let mut facts = Vec::new();
    collect_scope_facts(tree.root_node(), source.as_bytes(), path, "", &mut facts)?;
    Ok(facts)
}

fn scope_subject(path: &str, module_path: &str) -> String {
    if module_path.is_empty() {
        path.to_string()
    } else {
        format!("Module:{module_path}")
    }
}

fn qualify_name(module_path: &str, name: &str) -> String {
    if module_path.is_empty() {
        name.to_string()
    } else {
        format!("{module_path}::{name}")
    }
}

fn define_fact(path: &str, module_path: &str, kind: &str, name: &str) -> ExtractedFact {
    ExtractedFact {
        subject: scope_subject(path, module_path),
        predicate: "defines".to_string(),
        object: format!("{}:{}", kind, qualify_name(module_path, name)),
    }
}

fn collect_scope_facts(
    node: Node<'_>,
    source: &[u8],
    path: &str,
    module_path: &str,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    let mut cursor = node.walk();
    let mut pending_test_attribute = false;
    let mut scope_tests = Vec::new();
    let mut scope_functions = Vec::new();
    let mut seen_imports = HashSet::new();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "attribute_item" => {
                if is_test_attribute(child, source)? {
                    pending_test_attribute = true;
                }
                continue;
            }
            "line_comment" | "block_comment" => continue,
            "mod_item" => {
                if let Some(name) = child.child_by_field_name("name") {
                    let module_name = normalize_rust_identifier(name.utf8_text(source)?);
                    let nested_path = qualify_name(module_path, &module_name);
                    facts.push(ExtractedFact {
                        subject: scope_subject(path, module_path),
                        predicate: "defines".to_string(),
                        object: format!("Module:{nested_path}"),
                    });
                    if let Some(body) = child.child_by_field_name("body") {
                        collect_scope_facts(body, source, path, &nested_path, facts)?;
                    }
                }
            }
            "impl_item" => collect_impl_item_facts(child, source, path, module_path, facts)?,
            "function_item" => {
                if let Some(name) = child.child_by_field_name("name") {
                    let function = normalize_rust_identifier(name.utf8_text(source)?);
                    facts.push(define_fact(path, module_path, "Function", &function));
                    collect_call_facts(
                        child,
                        source,
                        &qualify_name(module_path, &function),
                        module_path,
                        facts,
                    )?;
                    if pending_test_attribute {
                        scope_tests.push(function);
                    } else {
                        scope_functions.push(function);
                    }
                }
                // Nested items (e.g. functions inside function bodies).
                collect_scope_facts(child, source, path, module_path, facts)?;
            }
            "struct_item" | "enum_item" | "trait_item" | "const_item" => {
                let kind = match child.kind() {
                    "struct_item" => "Struct",
                    "enum_item" => "Enum",
                    "trait_item" => "Trait",
                    _ => "Const",
                };
                if let Some(name) = child.child_by_field_name("name") {
                    let item_name = normalize_rust_identifier(name.utf8_text(source)?);
                    facts.push(define_fact(path, module_path, kind, &item_name));
                    if kind == "Enum" {
                        let qualified = qualify_name(module_path, &item_name);
                        let mut variants = Vec::new();
                        collect_enum_variants(child, source, &mut variants)?;
                        facts.extend(variants.into_iter().map(|variant| ExtractedFact {
                            subject: format!("Enum:{qualified}"),
                            predicate: "defines".to_string(),
                            object: format!("Variant:{qualified}::{variant}"),
                        }));
                    }
                }
            }
            "use_declaration" => {
                for import in expand_use_declaration_text(child.utf8_text(source)?) {
                    if seen_imports.insert(import.clone()) {
                        facts.push(ExtractedFact {
                            subject: scope_subject(path, module_path),
                            predicate: "imports".to_string(),
                            object: format!("Module:{import}"),
                        });
                    }
                }
            }
            _ => collect_scope_facts(child, source, path, module_path, facts)?,
        }
        if child.is_named() {
            pending_test_attribute = false;
        }
    }

    for (test, function) in map_tests_to_functions(&scope_tests, &scope_functions) {
        facts.push(ExtractedFact {
            subject: format!("Function:{}", qualify_name(module_path, &test)),
            predicate: "tests".to_string(),
            object: format!("Function:{}", qualify_name(module_path, &function)),
        });
    }
    Ok(())
}

fn collect_impl_item_facts(
    impl_node: Node<'_>,
    source: &[u8],
    path: &str,
    module_path: &str,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    let Some(type_node) = impl_node.child_by_field_name("type") else {
        return Ok(());
    };
    let type_name = qualify_module_type(&rust_type_base_name(type_node, source)?, module_path);

    if let Some(trait_node) = impl_node.child_by_field_name("trait") {
        let trait_name =
            qualify_module_type(&rust_type_base_name(trait_node, source)?, module_path);
        facts.push(ExtractedFact {
            subject: format!("Type:{type_name}"),
            predicate: "implements".to_string(),
            object: format!("Trait:{trait_name}"),
        });
    }

    let Some(body) = impl_node.child_by_field_name("body") else {
        return Ok(());
    };
    let mut cursor = body.walk();
    for item in body.children(&mut cursor) {
        match item.kind() {
            "function_item" => {
                if let Some(name) = item.child_by_field_name("name") {
                    let method = normalize_rust_identifier(name.utf8_text(source)?);
                    let qualified_method = format!("{type_name}::{method}");
                    facts.push(ExtractedFact {
                        subject: format!("Type:{type_name}"),
                        predicate: "defines".to_string(),
                        object: format!("Function:{qualified_method}"),
                    });
                    collect_call_facts(item, source, &qualified_method, module_path, facts)?;
                }
            }
            "attribute_item" | "line_comment" | "block_comment" => {}
            // Associated consts and other non-method items keep their
            // module-scoped identity.
            _ => collect_scope_facts(item, source, path, module_path, facts)?,
        }
    }
    Ok(())
}

/// Returns the base name of an impl-header type, stripping generic arguments
/// (`Trait<T>` -> `Trait`) while keeping scope qualifiers (`a::Trait`).
fn rust_type_base_name(node: Node<'_>, source: &[u8]) -> Result<String, Error> {
    if node.kind() == "generic_type"
        && let Some(base) = node.child_by_field_name("type")
    {
        return rust_type_base_name(base, source);
    }
    Ok(node.utf8_text(source)?.to_string())
}

fn collect_call_facts(
    function_node: Node<'_>,
    source: &[u8],
    qualified_caller: &str,
    module_path: &str,
    facts: &mut Vec<ExtractedFact>,
) -> Result<(), Error> {
    let mut calls = Vec::new();
    collect_calls(function_node, source, &mut calls)?;
    facts.extend(calls.into_iter().map(|callee| ExtractedFact {
        subject: format!("Function:{qualified_caller}"),
        predicate: "calls".to_string(),
        object: format!("Function:{}", qualify_module_call(&callee, module_path)),
    }));
    Ok(())
}

fn qualify_module_call(callee: &str, module_path: &str) -> String {
    if module_path.is_empty() || callee.contains("::") || callee.contains('.') {
        callee.to_string()
    } else {
        format!("{module_path}::{callee}")
    }
}

fn qualify_module_type(type_name: &str, module_path: &str) -> String {
    if module_path.is_empty() || type_name.contains("::") {
        type_name.to_string()
    } else {
        format!("{module_path}::{type_name}")
    }
}

fn map_tests_to_functions(tests: &[String], functions: &[String]) -> Vec<(String, String)> {
    let mut mappings = Vec::new();
    for test in tests {
        if let Some(function) = functions
            .iter()
            .filter(|function| test.starts_with(&format!("{function}_")))
            .max_by_key(|function| function.len())
        {
            mappings.push((test.clone(), function.clone()));
        }
    }
    mappings
}

fn collect_function_names(
    node: Node<'_>,
    source: &[u8],
    functions: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "function_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        functions.push(normalize_rust_identifier(name.utf8_text(source)?));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_function_names(child, source, functions)?;
    }
    Ok(())
}

fn normalize_rust_identifier(identifier: &str) -> String {
    identifier
        .strip_prefix("r#")
        .unwrap_or(identifier)
        .to_string()
}

fn collect_imports(node: Node<'_>, source: &[u8], imports: &mut Vec<String>) -> Result<(), Error> {
    if node.kind() == "use_declaration" {
        imports.extend(expand_use_declaration_text(node.utf8_text(source)?));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_imports(child, source, imports)?;
    }
    Ok(())
}

fn expand_use_declaration_text(declaration_text: &str) -> Vec<String> {
    let text = declaration_text
        .trim()
        .trim_start_matches("use ")
        .trim_end_matches(';')
        .trim();
    expand_rust_use_declaration(text)
}

fn expand_rust_use_declaration(declaration: &str) -> Vec<String> {
    let mut declaration = declaration;

    if let Some(without_pub) = declaration.strip_prefix("pub use ") {
        declaration = without_pub;
    } else if declaration.starts_with("pub(") {
        if let Some(close_paren) = declaration.find(") ") {
            declaration = &declaration[close_paren + 2..];
        }
        if let Some(without_use) = declaration.strip_prefix("use ") {
            declaration = without_use;
        }
    } else if let Some(without_use) = declaration.strip_prefix("pub ") {
        declaration = without_use;
    } else if let Some(without_use) = declaration.strip_prefix("use ") {
        declaration = without_use;
    }

    if let Some(inner) = declaration
        .strip_prefix('{')
        .and_then(|d| d.strip_suffix('}'))
    {
        let mut expanded = Vec::new();
        for item in split_top_level_commas(inner) {
            expanded.extend(expand_rust_use_declaration(item.trim()));
        }
        return expanded;
    }

    if let Some((prefix, rest)) = declaration.split_once("::{")
        && let Some(suffix_end) = rest.rfind('}')
    {
        let suffix = &rest[..suffix_end];
        let prefix = prefix.strip_prefix("::").unwrap_or(prefix);
        return expand_rust_use_items(prefix, suffix);
    }

    let declaration = declaration
        .split_once(" as ")
        .map_or(declaration, |(path, _)| path)
        .trim();
    let declaration = declaration.strip_suffix("::*").unwrap_or(declaration);
    let declaration = declaration.strip_prefix("::").unwrap_or(declaration);

    vec![normalize_rust_import_path(declaration)]
}

fn normalize_rust_import_path(path: &str) -> String {
    path.split("::")
        .map(|segment| segment.strip_prefix("r#").unwrap_or(segment))
        .collect::<Vec<_>>()
        .join("::")
}

fn expand_rust_use_items(prefix: &str, items: &str) -> Vec<String> {
    let mut expanded = Vec::new();

    for item in split_top_level_commas(items) {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }

        if let Some((nested_prefix, rest)) = item.split_once("::{")
            && let Some(suffix_end) = rest.rfind('}')
        {
            let nested_items = &rest[..suffix_end];
            let nested_prefix = nested_prefix.strip_prefix("::").unwrap_or(nested_prefix);
            let child_prefix = format!("{}::{}", prefix, nested_prefix);
            expanded.extend(expand_rust_use_items(&child_prefix, nested_items));
            continue;
        }

        let item = item
            .split_once(" as ")
            .map_or(item, |(item, _)| item)
            .trim();
        let item = item.strip_suffix("::*").unwrap_or(item);
        let item = item.strip_prefix("::").unwrap_or(item);

        if item == "self" {
            expanded.push(normalize_rust_import_path(prefix));
            continue;
        }

        if item == "*" {
            expanded.push(normalize_rust_import_path(prefix));
            continue;
        }

        expanded.push(normalize_rust_import_path(&format!("{}::{}", prefix, item)));
    }

    expanded
}

fn split_top_level_commas(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;

    for (idx, ch) in text.char_indices() {
        match ch {
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&text[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }

    parts.push(&text[start..]);
    parts
}

fn collect_calls(node: Node<'_>, source: &[u8], calls: &mut Vec<String>) -> Result<(), Error> {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
    {
        calls.push(function.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls(child, source, calls)?;
    }
    Ok(())
}

fn collect_structs(node: Node<'_>, source: &[u8], structs: &mut Vec<String>) -> Result<(), Error> {
    if node.kind() == "struct_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        structs.push(normalize_rust_identifier(name.utf8_text(source)?));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_structs(child, source, structs)?;
    }
    Ok(())
}

fn collect_enums(node: Node<'_>, source: &[u8], enums: &mut Vec<String>) -> Result<(), Error> {
    if node.kind() == "enum_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        enums.push(normalize_rust_identifier(name.utf8_text(source)?));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_enums(child, source, enums)?;
    }
    Ok(())
}

fn collect_traits(node: Node<'_>, source: &[u8], traits: &mut Vec<String>) -> Result<(), Error> {
    if node.kind() == "trait_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        traits.push(normalize_rust_identifier(name.utf8_text(source)?));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_traits(child, source, traits)?;
    }
    Ok(())
}

fn collect_modules(node: Node<'_>, source: &[u8], modules: &mut Vec<String>) -> Result<(), Error> {
    if node.kind() == "mod_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        modules.push(normalize_rust_identifier(name.utf8_text(source)?));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_modules(child, source, modules)?;
    }
    Ok(())
}

fn collect_consts(node: Node<'_>, source: &[u8], consts: &mut Vec<String>) -> Result<(), Error> {
    if node.kind() == "const_item"
        && let Some(name) = node.child_by_field_name("name")
    {
        consts.push(normalize_rust_identifier(name.utf8_text(source)?));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_consts(child, source, consts)?;
    }
    Ok(())
}

fn collect_tests(node: Node<'_>, source: &[u8], tests: &mut Vec<String>) -> Result<(), Error> {
    let mut cursor = node.walk();
    let mut pending_test_attribute = false;
    for child in node.children(&mut cursor) {
        match child.kind() {
            "attribute_item" => {
                if is_test_attribute(child, source)? {
                    pending_test_attribute = true;
                }
                continue;
            }
            "line_comment" | "block_comment" => continue,
            _ => {}
        }

        if pending_test_attribute
            && child.kind() == "function_item"
            && let Some(name) = child.child_by_field_name("name")
        {
            tests.push(normalize_rust_identifier(name.utf8_text(source)?));
        }

        collect_tests(child, source, tests)?;
        if child.is_named() {
            pending_test_attribute = false;
        }
    }
    Ok(())
}

/// Detects test marker attributes (`#[test]`, `#[tokio::test]`, `#[rstest]`,
/// `#[test_case]`, ...) by inspecting the last segment of the attribute path
/// instead of matching the literal text `#[test]`.
fn is_test_attribute(attribute_item: Node<'_>, source: &[u8]) -> Result<bool, Error> {
    let mut cursor = attribute_item.walk();
    for child in attribute_item.children(&mut cursor) {
        if child.kind() != "attribute" {
            continue;
        }
        let mut attribute_cursor = child.walk();
        for attribute_child in child.children(&mut attribute_cursor) {
            match attribute_child.kind() {
                "identifier" => {
                    return Ok(matches!(
                        attribute_child.utf8_text(source)?,
                        "test" | "rstest" | "test_case"
                    ));
                }
                "scoped_identifier" => {
                    let mut last_segment = None;
                    let mut segment_cursor = attribute_child.walk();
                    for segment in attribute_child.children(&mut segment_cursor) {
                        if segment.kind() == "identifier" {
                            last_segment = Some(segment);
                        }
                    }
                    return match last_segment {
                        Some(segment) => Ok(matches!(
                            segment.utf8_text(source)?,
                            "test" | "rstest" | "test_case"
                        )),
                        None => Ok(false),
                    };
                }
                _ => {}
            }
        }
    }
    Ok(false)
}

fn collect_enum_variants(
    node: Node<'_>,
    source: &[u8],
    variants: &mut Vec<String>,
) -> Result<(), Error> {
    if node.kind() == "enum_variant"
        && let Some(name) = node.child_by_field_name("name")
    {
        variants.push(name.utf8_text(source)?.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_enum_variants(child, source, variants)?;
    }
    Ok(())
}
