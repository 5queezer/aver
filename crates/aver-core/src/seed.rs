use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;

use crate::Error;
use crate::ontology::normalize_predicate_name;

const ENTITY_ONTOLOGY: &[(&str, Option<&str>)] = &[
    ("Thing", None),
    ("Asset", Some("Thing")),
    ("File", Some("Asset")),
    ("Module", Some("Asset")),
    ("Config", Some("Asset")),
    ("Symbol", Some("Thing")),
    ("Function", Some("Symbol")),
    ("Class", Some("Symbol")),
    ("Constant", Some("Symbol")),
    ("Process", Some("Thing")),
    ("Service", Some("Process")),
    ("Test", Some("Process")),
    ("Job", Some("Process")),
    ("Agent", Some("Thing")),
    ("Human", Some("Agent")),
    ("Bot", Some("Agent")),
    ("Concept", Some("Thing")),
    ("Decision", Some("Concept")),
    ("Bug", Some("Concept")),
    ("Pref", Some("Concept")),
    ("Constraint", Some("Concept")),
    // Agent-history entity types (ADR-0016)
    ("ClaudeSession", Some("Process")),
    ("ClaudeEvent", Some("Concept")),
    ("ClaudeContent", Some("Asset")),
    ("Project", Some("Asset")),
    ("ProjectPath", Some("File")),
    ("ClaudeHistory", Some("Asset")),
    ("ClaudeHistoryFile", Some("File")),
];

const PREDICATE_ONTOLOGY: &[(&str, Option<&str>)] = &[
    ("relates_to", None),
    ("depends_on", Some("relates_to")),
    ("calls", Some("depends_on")),
    ("imports", Some("depends_on")),
    ("reads_config_from", Some("depends_on")),
    ("owns", Some("relates_to")),
    ("owned_by", Some("owns")),
    ("authored", Some("owns")),
    ("maintained", Some("owns")),
    ("concerns", Some("relates_to")),
    ("fixes", Some("concerns")),
    ("tests", Some("concerns")),
    ("decides", Some("concerns")),
    // ADR-0018 M3: seat the historical kebab-case predicates (renamed to
    // snake_case) under sensible parents so the alias table resolves to a
    // real ontology entry.
    ("has_module", Some("owns")),
    ("binary_name", Some("owns")),
    ("impl_stays_in", Some("concerns")),
    ("splits_by", Some("relates_to")),
    ("uses", Some("depends_on")),
    // ADR-0018: predicates already emitted by the deterministic chat
    // extractor (`crates/aver-core/src/extractor.rs`). The extractor must
    // classify against the seeded vocabulary or fail; per ADR §"Test plan"
    // the test must extend the ontology in setup. Seating them here makes
    // the extractor's emitted vocabulary part of the seed.
    ("prefers", Some("concerns")),
    ("is", Some("relates_to")),
    ("has", Some("owns")),
    ("defines", Some("owns")),
    ("emits", Some("relates_to")),
    // ADR-0007 code edges the AST extractor emits (`aver-extractor` bridge):
    // inheritance and trait/interface implementation are dependencies of the
    // subtype on the supertype; `has_test` links a symbol to the test that
    // covers it. All multi-valued.
    ("extends", Some("depends_on")),
    ("implements", Some("depends_on")),
    ("has_test", Some("relates_to")),
    // ADR-0029: structured decision vocabulary. Domain predicates come from
    // the store's ontology.json extension (ADR-0031).
    ("triggered_by", Some("concerns")),
    ("supersedes", Some("relates_to")),
    ("has_rationale", Some("concerns")),
    // ADR-0032: evidence items name their context and when they closed; the
    // evidence-since-confirmation count keys on both.
    ("in_context", Some("relates_to")),
    ("closed_at", Some("relates_to")),
];

/// Predicates that hold one object per subject (migration 0010
/// `cardinality = 'single'`): consolidation supersedes the older object when
/// a newer claim disagrees. Every other predicate — `defines`, `calls`,
/// `imports`, `tests`, `uses`, `relates_to`, ... — accumulates objects, as do
/// predicates created by ontology extension.
const SINGLE_VALUED_PREDICATES: &[&str] = &[
    "prefers",
    "owned_by",
    "binary_name",
    "impl_stays_in",
    "splits_by",
    "closed_at",
];

const CORE_ALIASES: &[(&str, &str)] = &[("about", "concerns"), ("is_a", "is")];

/// Store-level ontology extension (ADR-0031): `<memory_dir>/ontology.json`.
/// It travels with the store and its backups, so open and replay see the same
/// vocabulary. Every section is optional; unknown keys are refused.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OntologyFile {
    version: u32,
    #[serde(default)]
    predicates: Vec<PredicateEntry>,
    #[serde(default)]
    entity_types: Vec<TypeEntry>,
    #[serde(default)]
    aliases: Vec<AliasEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PredicateEntry {
    name: String,
    parent: Option<String>,
    #[serde(default)]
    cardinality: Cardinality,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Cardinality {
    Single,
    #[default]
    Multi,
}

impl Cardinality {
    fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Multi => "multi",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TypeEntry {
    name: String,
    parent: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AliasEntry {
    alias: String,
    predicate: String,
}

pub(crate) const ONTOLOGY_FILE: &str = "ontology.json";

pub(crate) fn seed_ontology(conn: &Connection, memory_dir: &Path) -> Result<(), Error> {
    let extension = load_extension(memory_dir)?;
    seed_type_table(conn, "entity_types", ENTITY_ONTOLOGY)?;
    seed_type_table(conn, "predicate_types", PREDICATE_ONTOLOGY)?;
    for name in SINGLE_VALUED_PREDICATES {
        conn.execute(
            "UPDATE predicate_types SET cardinality = 'single' WHERE name = ?1",
            [name],
        )?;
    }
    for (alias, canonical) in CORE_ALIASES {
        conn.execute(
            "INSERT OR IGNORE INTO predicate_alias (alias, predicate_id, created_at, note)
             SELECT ?1, id, CAST(strftime('%s', 'now') AS INTEGER), 'structured decision vocabulary'
             FROM predicate_types WHERE name = ?2",
            params![alias, canonical],
        )?;
    }
    if let Some(extension) = extension {
        apply_extension(conn, &extension, &memory_dir.join(ONTOLOGY_FILE))?;
    }
    rebuild_closure(conn, "entity_types", "entity_type_closure")?;
    rebuild_closure(conn, "predicate_types", "predicate_closure")?;
    Ok(())
}

/// Absent file: core vocabulary only. Unreadable, malformed or unknown-version
/// file: fail closed so a store never runs with half its vocabulary.
fn load_extension(memory_dir: &Path) -> Result<Option<OntologyFile>, Error> {
    let path = memory_dir.join(ONTOLOGY_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ontology_error(&path, format!("cannot read: {error}"))),
    };
    let file: OntologyFile = serde_json::from_str(&text).map_err(|error| {
        ontology_error(
            &path,
            format!(
                "{error}; expected {{\"version\":1,\"predicates\":[{{\"name\",\"parent\",\"cardinality\"}}],\"entity_types\":[{{\"name\",\"parent\"}}],\"aliases\":[{{\"alias\",\"predicate\"}}]}}"
            ),
        )
    })?;
    if file.version != 1 {
        return Err(ontology_error(
            &path,
            format!(
                "unsupported version {}; this binary reads version 1",
                file.version
            ),
        ));
    }
    Ok(Some(file))
}

fn ontology_error(path: &Path, detail: String) -> Error {
    Error::OntologyFile {
        path: path.display().to_string(),
        detail,
    }
}

/// Additive and idempotent: inserts what is missing and pins each declared
/// entry's parent (and cardinality). It never changes a core entry and never
/// deletes rows, so vocabulary added by an older seed or file stays usable.
fn apply_extension(conn: &Connection, file: &OntologyFile, path: &Path) -> Result<(), Error> {
    let fail = |entry: String, detail: String| ontology_error(path, format!("{entry}: {detail}"));
    let core_predicates: HashMap<&str, Option<&str>> = PREDICATE_ONTOLOGY.iter().copied().collect();
    let mut declared = HashSet::new();
    for (i, entry) in file.predicates.iter().enumerate() {
        let label = format!("predicates[{i}] {:?}", entry.name);
        if entry.name.is_empty() || normalize_predicate_name(&entry.name) != entry.name {
            return Err(fail(label, "name must be lowercase snake_case".into()));
        }
        if let Some(core_parent) = core_predicates.get(entry.name.as_str()) {
            let core_cardinality = if SINGLE_VALUED_PREDICATES.contains(&entry.name.as_str()) {
                Cardinality::Single
            } else {
                Cardinality::Multi
            };
            if entry.parent.as_deref() != *core_parent || entry.cardinality != core_cardinality {
                return Err(fail(
                    label,
                    "is a core predicate; its parent and cardinality cannot change (drop the entry)"
                        .into(),
                ));
            }
            continue;
        }
        if let Some(parent) = &entry.parent
            && !core_predicates.contains_key(parent.as_str())
            && !declared.contains(parent.as_str())
        {
            return Err(fail(
                label,
                format!("parent {parent:?} is neither a core predicate nor an earlier entry"),
            ));
        }
        conn.execute(
            "INSERT OR IGNORE INTO predicate_types (name) VALUES (?1)",
            [&entry.name],
        )?;
        conn.execute(
            "UPDATE predicate_types
                SET parent_id = (SELECT id FROM predicate_types WHERE name = ?2), cardinality = ?3
              WHERE name = ?1",
            params![entry.name, entry.parent, entry.cardinality.as_str()],
        )?;
        declared.insert(entry.name.as_str());
    }
    let core_types: HashMap<&str, Option<&str>> = ENTITY_ONTOLOGY.iter().copied().collect();
    let mut declared_types = HashSet::new();
    for (i, entry) in file.entity_types.iter().enumerate() {
        let label = format!("entity_types[{i}] {:?}", entry.name);
        if entry.name.is_empty()
            || entry
                .name
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(fail(
                label,
                "name must be non-empty without whitespace".into(),
            ));
        }
        if let Some(core_parent) = core_types.get(entry.name.as_str()) {
            if entry.parent.as_deref() != *core_parent {
                return Err(fail(
                    label,
                    "is a core entity type; its parent cannot change (drop the entry)".into(),
                ));
            }
            continue;
        }
        if let Some(parent) = &entry.parent
            && !core_types.contains_key(parent.as_str())
            && !declared_types.contains(parent.as_str())
        {
            return Err(fail(
                label,
                format!("parent {parent:?} is neither a core entity type nor an earlier entry"),
            ));
        }
        conn.execute(
            "INSERT OR IGNORE INTO entity_types (name) VALUES (?1)",
            [&entry.name],
        )?;
        conn.execute(
            "UPDATE entity_types SET parent_id = (SELECT id FROM entity_types WHERE name = ?2)
              WHERE name = ?1",
            params![entry.name, entry.parent],
        )?;
        declared_types.insert(entry.name.as_str());
    }
    for (i, entry) in file.aliases.iter().enumerate() {
        let label = format!("aliases[{i}] {:?}", entry.alias);
        if entry.alias.is_empty() || normalize_predicate_name(&entry.alias) != entry.alias {
            return Err(fail(label, "alias must be lowercase snake_case".into()));
        }
        if !core_predicates.contains_key(entry.predicate.as_str())
            && !declared.contains(entry.predicate.as_str())
        {
            return Err(fail(
                label,
                format!(
                    "predicate {:?} is neither a core predicate nor declared in this file",
                    entry.predicate
                ),
            ));
        }
        let is_predicate: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM predicate_types WHERE name = ?1)",
            [&entry.alias],
            |row| row.get(0),
        )?;
        let target: Option<String> = conn
            .query_row(
                "SELECT predicate_types.name FROM predicate_alias
                   JOIN predicate_types ON predicate_types.id = predicate_alias.predicate_id
                  WHERE predicate_alias.alias = ?1",
                [&entry.alias],
                |row| row.get(0),
            )
            .optional()?;
        if is_predicate || target.as_ref().is_some_and(|t| *t != entry.predicate) {
            return Err(fail(
                label,
                "already names another predicate or alias; choose a different alias".into(),
            ));
        }
        conn.execute(
            "INSERT OR IGNORE INTO predicate_alias (alias, predicate_id, created_at, note)
             SELECT ?1, id, CAST(strftime('%s', 'now') AS INTEGER), 'ontology.json'
             FROM predicate_types WHERE name = ?2",
            params![entry.alias, entry.predicate],
        )?;
    }
    Ok(())
}

pub(crate) fn seed_type_table(
    conn: &Connection,
    table: &str,
    ontology: &[(&str, Option<&str>)],
) -> Result<(), Error> {
    for (name, _parent) in ontology {
        conn.execute(
            &format!("INSERT OR IGNORE INTO {table} (name) VALUES (?1)"),
            [name],
        )?;
    }
    for (name, parent) in ontology {
        let parent_id = if let Some(parent) = parent {
            conn.query_row(
                &format!("SELECT id FROM {table} WHERE name = ?1"),
                [parent],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        } else {
            None
        };
        conn.execute(
            &format!("UPDATE {table} SET parent_id = ?2 WHERE name = ?1"),
            params![name, parent_id],
        )?;
    }
    Ok(())
}

pub(crate) fn rebuild_closure(
    conn: &Connection,
    type_table: &str,
    closure_table: &str,
) -> Result<(), Error> {
    conn.execute(&format!("DELETE FROM {closure_table}"), [])?;
    let mut stmt = conn.prepare(&format!("SELECT id, parent_id FROM {type_table}"))?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?))
    })?;
    let mut parents = HashMap::new();
    for row in rows {
        let (id, parent_id) = row?;
        parents.insert(id, parent_id);
    }
    for child_id in parents.keys().copied() {
        let mut ancestor = Some(child_id);
        while let Some(ancestor_id) = ancestor {
            conn.execute(
                &format!(
                    "INSERT OR IGNORE INTO {closure_table} (child_id, ancestor_id) VALUES (?1, ?2)"
                ),
                params![child_id, ancestor_id],
            )?;
            ancestor = parents.get(&ancestor_id).copied().flatten();
        }
    }
    Ok(())
}
