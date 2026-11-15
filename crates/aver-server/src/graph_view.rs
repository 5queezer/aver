//! Read-only graph snapshot for the live memory view.
//!
//! Mirrors `contrib/graph/export.py` (format `aver-graph-v1`) so the page
//! renders a served snapshot exactly like a manual export. The store is opened
//! read-only, never created, and every query runs inside one read transaction.

use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::{Connection, OpenFlags, Row};
use serde_json::{Map, Value, json};

/// Build the `aver-graph-v1` snapshot of `db_path` without writing to it.
pub fn snapshot(db_path: &Path) -> anyhow::Result<Value> {
    let mut conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    let tx = conn.transaction()?;

    let columns = column_names(&tx, "claims")?;
    let kind = if columns.contains("object_kind") {
        "object_kind"
    } else {
        "'entity' AS object_kind"
    };
    let event = if columns.contains("event_id") {
        "event_id"
    } else {
        "NULL AS event_id"
    };
    let created = if columns.contains("created_at") {
        "created_at"
    } else {
        "NULL AS created_at"
    };
    let mut claims = rows(
        &tx,
        &format!(
            "SELECT id, subject, predicate, object, provenance, confidence, status, \
             source_refs, scope, {kind}, {event}, {created} FROM claims ORDER BY id"
        ),
    )?;
    for claim in &mut claims {
        let refs: Value = serde_json::from_str(claim["source_refs"].as_str().unwrap_or("[]"))?;
        // Preserve the old candidate workflow's provenance references too.
        let mut event_ids: BTreeSet<i64> = refs
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| r.as_str()?.strip_prefix("event:"))
            .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
            .filter_map(|id| id.parse().ok())
            .collect();
        event_ids.extend(claim["event_id"].as_i64());
        claim.insert("source_refs".into(), refs);
        claim.insert("event_ids".into(), json!(event_ids));
    }

    let tables = rows(&tx, "SELECT name FROM sqlite_master WHERE type='table'")?
        .into_iter()
        .filter_map(|r| r["name"].as_str().map(str::to_string))
        .collect::<BTreeSet<_>>();
    let events = if tables.contains("episodic_events") {
        rows(
            &tx,
            "SELECT id, session_id, kind, payload, source, agent_id, agent_kind, ts, scope \
             FROM episodic_events ORDER BY id",
        )?
    } else {
        Vec::new()
    };
    let contradictions = if tables.contains("contradictions") {
        rows(
            &tx,
            "SELECT id, claim_id, reason, new_claim_id FROM contradictions ORDER BY id",
        )?
    } else {
        Vec::new()
    };
    // The ontology declares cardinality, not object kind (migration 0010).
    let predicates = if tables.contains("predicate_types") {
        let cardinality = if column_names(&tx, "predicate_types")?.contains("cardinality") {
            "p.cardinality"
        } else {
            "'multi'"
        };
        rows(
            &tx,
            &format!(
                "SELECT p.name, parent.name AS parent, {cardinality} AS cardinality \
                 FROM predicate_types p LEFT JOIN predicate_types parent \
                 ON parent.id = p.parent_id ORDER BY p.name"
            ),
        )?
    } else {
        Vec::new()
    };

    let mut names = BTreeSet::new();
    for claim in &claims {
        names.extend(claim["subject"].as_str().map(str::to_string));
        if claim["object_kind"] == "entity" {
            names.extend(claim["object"].as_str().map(str::to_string));
        }
    }
    let entities: Vec<Value> = names
        .into_iter()
        .map(|n| json!({"id": n, "label": n}))
        .collect();
    Ok(json!({
        "format": "aver-graph-v1",
        "entities": entities,
        "claims": claims,
        "events": events,
        "contradictions": contradictions,
        "predicates": predicates,
    }))
}

fn column_names(conn: &Connection, table: &str) -> rusqlite::Result<BTreeSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    stmt.query_map([], |r| r.get::<_, String>(1))?.collect()
}

fn rows(conn: &Connection, sql: &str) -> rusqlite::Result<Vec<Map<String, Value>>> {
    let mut stmt = conn.prepare(sql)?;
    let names: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
    stmt.query_map([], |row| row_object(row, &names))?.collect()
}

fn row_object(row: &Row<'_>, names: &[String]) -> rusqlite::Result<Map<String, Value>> {
    use rusqlite::types::ValueRef;
    let mut object = Map::new();
    for (i, name) in names.iter().enumerate() {
        let value = match row.get_ref(i)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(n) => json!(n),
            ValueRef::Real(f) => json!(f),
            ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
            ValueRef::Blob(_) => Value::Null,
        };
        object.insert(name.clone(), value);
    }
    Ok(object)
}
