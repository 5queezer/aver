#!/usr/bin/env python3
"""Export an explicit local Aver projection to a new private JSON file.

Uses only the standard library. The input database is opened read-only and all
queries share one snapshot. Generated exports can contain private memory: keep
them outside the repository and load them locally in graph.html.
"""
import argparse
import contextlib
import json
import os
from pathlib import Path
import sqlite3


def export(database):
    uri = Path(database).resolve().as_uri() + "?mode=ro"
    with contextlib.closing(sqlite3.connect(uri, uri=True)) as conn:
        conn.row_factory = sqlite3.Row
        conn.execute("BEGIN")
        columns = {r[1] for r in conn.execute("PRAGMA table_info(claims)")}
        kind = "object_kind" if "object_kind" in columns else "'entity' AS object_kind"
        event = "event_id" if "event_id" in columns else "NULL AS event_id"
        created = "created_at" if "created_at" in columns else "NULL AS created_at"
        claims = [dict(r) for r in conn.execute(
            "SELECT id, subject, predicate, object, provenance, confidence, status, "
            f"source_refs, scope, {kind}, {event}, {created} FROM claims ORDER BY id"
        )]
        for claim in claims:
            claim["source_refs"] = json.loads(claim["source_refs"])
            # Preserve the old candidate workflow's provenance references too.
            claim["event_ids"] = sorted({int(ref[6:]) for ref in claim["source_refs"]
                                         if isinstance(ref, str) and ref.startswith("event:")
                                         and ref[6:].isdigit()}
                                        | ({claim["event_id"]} if claim["event_id"] is not None else set()))
        tables = {r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        events = [dict(r) for r in conn.execute(
            "SELECT id, session_id, kind, payload, source, agent_id, agent_kind, ts, scope "
            "FROM episodic_events ORDER BY id"
        )] if "episodic_events" in tables else []
        contradictions = [dict(r) for r in conn.execute(
            "SELECT id, claim_id, reason, new_claim_id FROM contradictions ORDER BY id"
        )] if "contradictions" in tables else []
        # The ontology declares cardinality, not object kind (migration 0010).
        predicate_columns = {r[1] for r in conn.execute("PRAGMA table_info(predicate_types)")}
        cardinality = "p.cardinality" if "cardinality" in predicate_columns else "'multi'"
        predicates = [dict(r) for r in conn.execute(
            f"SELECT p.name, parent.name AS parent, {cardinality} AS cardinality FROM predicate_types p "
            "LEFT JOIN predicate_types parent ON parent.id = p.parent_id ORDER BY p.name"
        )] if "predicate_types" in tables else []
        names = {c["subject"] for c in claims}
        names.update(c["object"] for c in claims if c["object_kind"] == "entity")
        entities = [{"id": n, "label": n} for n in sorted(names)]
        return {"format": "aver-graph-v1", "entities": entities,
                "claims": claims, "events": events, "contradictions": contradictions,
                "predicates": predicates}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("database", type=Path, help="Aver db.sqlite (read-only)")
    parser.add_argument("output", type=Path, help="new private JSON file, outside the repository")
    args = parser.parse_args()
    data = export(args.database)
    # Exclusive creation preserves existing exports; restrict access even when
    # the caller has a permissive umask. Do not print memory text or paths.
    fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as out:
        json.dump(data, out, ensure_ascii=False)
        out.write("\n")
    print(json.dumps({key: len(data[key]) for key in ("entities", "claims", "events", "contradictions", "predicates")}))


if __name__ == "__main__":
    main()
