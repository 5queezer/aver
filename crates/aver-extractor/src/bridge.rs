//! Bridge from the extractor's [`ExtractedFact`] to core's [`ExtractorFact`]
//! (ADR-0027). One conversion, shared by `aver extract` and the planned
//! `extract` MCP tool.
//!
//! Entity names are rewritten onto core's seeded entity types so core types
//! them instead of filing them under `Thing` for review, which gives the
//! ADR-0007 shapes `File defines Symbol`, `Symbol calls Symbol` and
//! `Module imports Module`. Predicates outside core's seeded ontology are
//! skipped and counted: core rejects an unknown predicate on an `EXTRACTED`
//! write.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use aver_core::{ExtractorFact, Provenance};

use crate::{Error, ExtractedFact, extract_facts_for_path, language_for_path};

/// Extractor predicates that core's seeded ontology accepts. The extractor
/// also emits `extends` and `implements`, and ADR-0007 names `has_test`;
/// none of the three is seeded, so those facts are skipped.
pub const SUPPORTED_PREDICATES: &[&str] = &["defines", "calls", "imports", "tests", "uses"];

/// Directory names never descended into by [`extract_tree`], in addition to
/// hidden (dot) entries.
const SKIPPED_DIRS: &[&str] = &["target", "node_modules"];

/// Convert one fact extracted from the file at `path`. Returns `None` when
/// the predicate is not in [`SUPPORTED_PREDICATES`].
pub fn to_core_fact(path: &str, fact: &ExtractedFact) -> Option<ExtractorFact> {
    let predicate = SUPPORTED_PREDICATES
        .iter()
        .find(|predicate| **predicate == fact.predicate)?;
    Some(
        ExtractorFact::new(
            canonical_entity(path, &fact.subject),
            *predicate,
            canonical_entity(path, &fact.object),
        )
        .with_provenance(Provenance::Extracted),
    )
}

/// `path` itself becomes `File:<path>`; `Kind:name` keeps its name and maps
/// the kind onto a seeded entity type; anything else becomes `Symbol:<raw>`.
fn canonical_entity(path: &str, raw: &str) -> String {
    if raw == path {
        return format!("File:{path}");
    }
    match raw.split_once(':') {
        Some((kind, name))
            if !kind.is_empty()
                && kind.bytes().all(|byte| byte.is_ascii_alphabetic())
                && !name.starts_with(':') =>
        {
            format!("{}:{name}", entity_type(kind))
        }
        _ => format!("Symbol:{raw}"),
    }
}

/// Map an extractor kind onto core's seeded entity types (`seed.rs`).
fn entity_type(kind: &str) -> &'static str {
    match kind {
        "File" => "File",
        "Function" => "Function",
        "Class" | "Struct" | "Enum" | "Interface" | "Trait" | "Protocol" | "Record" | "Object"
        | "Actor" | "Type" | "Annotation" => "Class",
        "Module" | "Namespace" | "Package" => "Module",
        "Const" | "Constant" => "Constant",
        // Variant, TypeAlias, Delegate and any kind added later.
        _ => "Symbol",
    }
}

/// Converted facts for one source file. `path` is the claim source reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFacts {
    pub path: String,
    pub facts: Vec<ExtractorFact>,
}

/// Result of [`extract_tree`].
#[derive(Debug, Default)]
pub struct TreeExtraction {
    pub files: Vec<FileFacts>,
    /// Facts the extractor produced, before conversion.
    pub extracted: usize,
    /// Facts dropped because the same converted triple was already produced
    /// earlier in this run (same file or an earlier one), so one run never
    /// writes a triple twice. The claim keeps the first file as its source.
    pub duplicates: usize,
    /// Unsupported facts, counted by extractor predicate.
    pub skipped: BTreeMap<String, usize>,
    /// Files that could not be read or parsed, with the error.
    pub failed: Vec<(String, Error)>,
}

impl TreeExtraction {
    pub fn files_scanned(&self) -> usize {
        self.files.len() + self.failed.len()
    }

    pub fn fact_count(&self) -> usize {
        self.files.iter().map(|file| file.facts.len()).sum()
    }

    pub fn facts_by_predicate(&self) -> BTreeMap<&str, usize> {
        let mut counts = BTreeMap::new();
        for fact in self.files.iter().flat_map(|file| &file.facts) {
            *counts.entry(fact.predicate.as_str()).or_default() += 1;
        }
        counts
    }
}

/// Extract and convert every supported source file under `root` (or `root`
/// itself when it is a file), in path order. Hidden entries, `target` and
/// `node_modules` are skipped and symlinks are not followed. `lang` limits
/// the scan to one [`crate::LANGUAGES`] entry.
pub fn extract_tree(root: &Path, lang: Option<&str>) -> Result<TreeExtraction, Error> {
    let mut paths = Vec::new();
    if std::fs::metadata(root)?.is_dir() {
        collect_files(root, &mut paths)?;
    } else {
        paths.push(root.to_path_buf());
    }

    let mut out = TreeExtraction::default();
    let mut seen = HashSet::new();
    for file in paths {
        let path = file.to_string_lossy();
        let path = path.strip_prefix("./").unwrap_or(&path).to_string();
        let Some(language) = language_for_path(&path) else {
            continue;
        };
        if lang.is_some_and(|lang| lang != language) {
            continue;
        }
        let extracted = std::fs::read_to_string(&file)
            .map_err(Error::from)
            .and_then(|source| extract_facts_for_path(&path, &source));
        let extracted = match extracted {
            Ok(facts) => facts,
            Err(err) => {
                out.failed.push((path, err));
                continue;
            }
        };
        out.extracted += extracted.len();
        let mut facts = Vec::new();
        for fact in &extracted {
            match to_core_fact(&path, fact) {
                Some(converted) => {
                    let key = (
                        converted.subject.clone(),
                        converted.predicate.clone(),
                        converted.object.clone(),
                    );
                    if seen.insert(key) {
                        facts.push(converted);
                    } else {
                        out.duplicates += 1;
                    }
                }
                None => *out.skipped.entry(fact.predicate.clone()).or_default() += 1,
            }
        }
        out.files.push(FileFacts { path, facts });
    }
    Ok(out)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Error> {
    let mut entries = std::fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || SKIPPED_DIRS.contains(&name.as_ref()) {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files(&entry.path(), out)?;
        } else if file_type.is_file() {
            out.push(entry.path());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triples(path: &str, source: &str) -> Vec<(String, String, String)> {
        extract_facts_for_path(path, source)
            .unwrap()
            .iter()
            .filter_map(|fact| to_core_fact(path, fact))
            .map(|fact| (fact.subject, fact.predicate, fact.object))
            .collect()
    }

    fn has(triples: &[(String, String, String)], s: &str, p: &str, o: &str) -> bool {
        triples
            .iter()
            .any(|(subject, predicate, object)| subject == s && predicate == p && object == o)
    }

    #[test]
    fn converted_facts_are_extracted_provenance() {
        let fact = ExtractedFact {
            subject: "src/lib.rs".into(),
            predicate: "defines".into(),
            object: "Function:run".into(),
        };
        let converted = to_core_fact("src/lib.rs", &fact).unwrap();
        assert_eq!(converted.subject, "File:src/lib.rs");
        assert_eq!(converted.object, "Function:run");
        assert_eq!(converted.provenance, Some(Provenance::Extracted));
    }

    #[test]
    fn rust_functions_calls_and_imports_map_to_adr_0007_shapes() {
        let t = triples(
            "src/lib.rs",
            "use std::collections::HashMap;\nfn helper() {}\nfn run() { helper(); }\n",
        );
        assert!(has(&t, "File:src/lib.rs", "defines", "Function:run"));
        assert!(has(&t, "Function:run", "calls", "Function:helper"));
        assert!(has(
            &t,
            "File:src/lib.rs",
            "imports",
            "Module:std::collections::HashMap"
        ));
    }

    #[test]
    fn rust_types_and_impl_methods_map_to_class() {
        let t = triples(
            "src/lib.rs",
            "pub struct Store;\npub enum Mode { A }\npub trait Open {}\n\
             impl Store { pub fn open() {} }\nconst LIMIT: u8 = 1;\n",
        );
        assert!(has(&t, "File:src/lib.rs", "defines", "Class:Store"));
        assert!(has(&t, "File:src/lib.rs", "defines", "Class:Mode"));
        assert!(has(&t, "File:src/lib.rs", "defines", "Class:Open"));
        assert!(has(&t, "File:src/lib.rs", "defines", "Constant:LIMIT"));
        assert!(has(&t, "Class:Mode", "defines", "Symbol:Mode::A"));
        assert!(has(&t, "Class:Store", "defines", "Function:Store::open"));
    }

    #[test]
    fn rust_tests_keep_the_seeded_tests_predicate() {
        let t = triples(
            "src/lib.rs",
            "fn parse() {}\n#[test]\nfn parse_works() { parse(); }\n",
        );
        assert!(has(&t, "Function:parse_works", "tests", "Function:parse"));
    }

    #[test]
    fn rust_modules_are_modules() {
        let t = triples("src/lib.rs", "mod inner { fn f() {} }\n");
        assert!(has(&t, "File:src/lib.rs", "defines", "Module:inner"));
        assert!(has(&t, "Module:inner", "defines", "Function:inner::f"));
    }

    #[test]
    fn php_trait_use_keeps_uses() {
        let t = triples(
            "app/User.php",
            "<?php\ntrait Greets {}\nclass User { use Greets; }\n",
        );
        assert!(has(&t, "Class:User", "uses", "Class:Greets"));
    }

    #[test]
    fn extends_and_implements_are_skipped() {
        for (predicate, source, path) in [
            (
                "extends",
                "class Base:\n    pass\nclass Child(Base):\n    pass\n",
                "m.py",
            ),
            (
                "implements",
                "trait T {}\nstruct S;\nimpl T for S {}\n",
                "m.rs",
            ),
        ] {
            let facts = extract_facts_for_path(path, source).unwrap();
            let fact = facts
                .iter()
                .find(|fact| fact.predicate == predicate)
                .unwrap_or_else(|| panic!("fixture must emit {predicate}"));
            assert_eq!(to_core_fact(path, fact), None);
        }
    }

    #[test]
    fn kinds_map_onto_seeded_entity_types() {
        for (raw, expected) in [
            ("Interface:I", "Class:I"),
            ("Protocol:P", "Class:P"),
            ("Record:R", "Class:R"),
            ("Object:O", "Class:O"),
            ("Actor:A", "Class:A"),
            ("Namespace:N", "Module:N"),
            ("Package:com.x", "Module:com.x"),
            ("TypeAlias:T", "Symbol:T"),
            ("Delegate:D", "Symbol:D"),
            ("Variant:E::V", "Symbol:E::V"),
            ("a::b", "Symbol:a::b"),
            ("bare", "Symbol:bare"),
        ] {
            assert_eq!(canonical_entity("x.rs", raw), expected, "{raw}");
        }
    }

    #[test]
    fn every_supported_predicate_and_mapped_kind_is_accepted_by_core() {
        let dir = tempfile::tempdir().unwrap();
        let store = aver_core::Store::open(dir.path()).unwrap();
        let path = "src/lib.rs";
        let facts: Vec<_> = extract_facts_for_path(
            path,
            "use std::fmt;\nmod m { pub fn f() {} }\npub struct S;\npub enum E { V }\n\
             impl S { fn g() { m::f(); } }\nconst C: u8 = 1;\nfn f() {}\n\
             #[test]\nfn f_works() { f(); }\n",
        )
        .unwrap()
        .iter()
        .filter_map(|fact| to_core_fact(path, fact))
        .collect();
        let mut predicates: Vec<_> = facts.iter().map(|f| f.predicate.as_str()).collect();
        predicates.sort();
        predicates.dedup();
        assert_eq!(predicates, ["calls", "defines", "imports", "tests"]);

        store
            .ingest_extractor_facts(
                "aver_extractor",
                aver_core::AgentKind::DeterministicParser,
                path,
                &facts,
            )
            .unwrap();
        store
            .ingest_extractor_facts(
                "aver_extractor",
                aver_core::AgentKind::DeterministicParser,
                "app/User.php",
                &[to_core_fact(
                    "app/User.php",
                    &ExtractedFact {
                        subject: "Class:User".into(),
                        predicate: "uses".into(),
                        object: "Trait:Greets".into(),
                    },
                )
                .unwrap()],
            )
            .unwrap();
        // Every entity resolved to a seeded type (the `Symbol` fallback is
        // seeded too), so none is queued for review as `Thing`.
        assert_eq!(store.requires_review_count().unwrap(), 0);
    }

    #[test]
    fn extract_tree_walks_sorted_supported_files_and_counts_skips() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(
            root.join("src/b.py"),
            "class A:\n    pass\nclass B(A):\n    pass\n",
        )
        .unwrap();
        std::fs::write(root.join("src/a.rs"), "fn a() {}\nfn b() { a(); a(); }\n").unwrap();
        // Same `b calls a` triple from a second file.
        std::fs::write(root.join("src/c.rs"), "fn b() { a(); }\n").unwrap();
        std::fs::write(root.join(".hidden/c.rs"), "fn c() {}\n").unwrap();
        std::fs::write(root.join("target/d.rs"), "fn d() {}\n").unwrap();
        std::fs::write(root.join("notes.txt"), "not code").unwrap();

        let all = extract_tree(root, None).unwrap();
        let names: Vec<_> = all
            .files
            .iter()
            .map(|file| Path::new(&file.path).file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["a.rs", "b.py", "c.rs"]);
        assert_eq!(all.files_scanned(), 3);
        assert_eq!(all.skipped.get("extends"), Some(&1));
        assert_eq!(
            all.duplicates, 2,
            "`b calls a` is written once per run, within and across files"
        );
        assert_eq!(
            all.extracted,
            all.fact_count() + all.duplicates + all.skipped.values().sum::<usize>()
        );
        assert_eq!(all.facts_by_predicate().get("calls"), Some(&1));

        let rust_only = extract_tree(root, Some("rust")).unwrap();
        assert_eq!(rust_only.files.len(), 2);
        assert!(rust_only.skipped.is_empty());
    }
}
