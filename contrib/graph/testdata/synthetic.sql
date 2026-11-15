-- Invented memory for exporter parity tests (Python export.py and Rust
-- aver-server --graph-view must both produce synthetic.json from it).
CREATE TABLE claims(id INTEGER PRIMARY KEY, subject TEXT, predicate TEXT, object TEXT,
    provenance TEXT, confidence REAL, status TEXT, source_refs TEXT, scope TEXT,
    object_kind TEXT, event_id INTEGER, created_at INTEGER);
CREATE TABLE episodic_events(id INTEGER PRIMARY KEY, session_id TEXT, kind TEXT, payload TEXT,
    source TEXT, agent_id TEXT, agent_kind TEXT, ts INTEGER, scope TEXT);
CREATE TABLE contradictions(id INTEGER PRIMARY KEY, claim_id INTEGER, reason TEXT, new_claim_id INTEGER);
CREATE TABLE predicate_types(id INTEGER PRIMARY KEY, name TEXT, parent_id INTEGER, cardinality TEXT);
INSERT INTO predicate_types VALUES
    (1, 'relates_to', NULL, 'multi'),
    (2, 'about', 1, 'multi'),
    (3, 'has_outcome', 1, 'single');
INSERT INTO episodic_events VALUES
    (1, 'synthetic-session', 'decision', 'Invented decision text <b>not markup</b>', 'synthetic', 'model-a', 'LLM', 1700000000, 'proj/example'),
    (2, 'synthetic-session', 'note', 'Second invented event', 'synthetic', 'model-a', 'LLM', 1700000100, 'proj/example');
INSERT INTO claims VALUES
    (1, 'decision:alpha', 'about', 'topic:example', 'INFERRED', 0.45, 'ACTIVE', '[]', 'proj/example', 'entity', 1, 1700000010),
    (2, 'decision:alpha', 'has_rationale', 'An invented rationale sentence.', 'INFERRED', 0.7, 'ACTIVE', '["event:2","note:x"]', 'proj/example', 'literal', NULL, 1700000110),
    (3, 'decision:beta', 'supersedes', 'decision:alpha', 'EXTRACTED', 0.9, 'ACTIVE', '["event:1"]', 'proj/example', 'entity', 1, 1700000200),
    (4, 'decision:alpha', 'has_outcome', 'pending', 'INFERRED', 0.45, 'INVALIDATED', '["event:","retired:invented reason"]', 'proj/example', 'literal', NULL, 1700000300);
INSERT INTO contradictions VALUES (1, 4, 'invented conflicting evidence', 3);
