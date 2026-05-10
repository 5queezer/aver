use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use crate::Error;

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
];

pub(crate) fn seed_ontology(conn: &Connection) -> Result<(), Error> {
    seed_type_table(conn, "entity_types", ENTITY_ONTOLOGY)?;
    seed_type_table(conn, "predicate_types", PREDICATE_ONTOLOGY)?;
    rebuild_closure(conn, "entity_types", "entity_type_closure")?;
    rebuild_closure(conn, "predicate_types", "predicate_closure")?;
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
