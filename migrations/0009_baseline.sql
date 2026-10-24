-- Baseline for fresh stores: verbatim concatenation of legacy migrations
-- 0009..0119, which live in crates/aver-core/migrations-legacy/ and are
-- applied only to stores created before this baseline.
-- Do not edit; add new migrations as 0010_*.sql.

-- ==== legacy 0009_value_range_checks.sql ====
CREATE TRIGGER IF NOT EXISTS claims_confidence_range_insert
BEFORE INSERT ON claims
WHEN NEW.confidence < 0.0 OR NEW.confidence > 1.0
BEGIN
  SELECT RAISE(ABORT, 'claims.confidence must be in [0, 1]');
END;

CREATE TRIGGER IF NOT EXISTS claims_confidence_range_update
BEFORE UPDATE OF confidence ON claims
WHEN NEW.confidence < 0.0 OR NEW.confidence > 1.0
BEGIN
  SELECT RAISE(ABORT, 'claims.confidence must be in [0, 1]');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_confidence_range_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.confidence < 0.0 OR NEW.confidence > 1.0
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.confidence must be in [0, 1]');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_confidence_range_update
BEFORE UPDATE OF confidence ON candidate_claims
WHEN NEW.confidence < 0.0 OR NEW.confidence > 1.0
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.confidence must be in [0, 1]');
END;

CREATE TRIGGER IF NOT EXISTS claims_status_enum_insert
BEFORE INSERT ON claims
WHEN NEW.status NOT IN ('ACTIVE', 'SUPERSEDED', 'INVALIDATED')
BEGIN
  SELECT RAISE(ABORT, 'claims.status must be ACTIVE, SUPERSEDED, or INVALIDATED');
END;

CREATE TRIGGER IF NOT EXISTS claims_status_enum_update
BEFORE UPDATE OF status ON claims
WHEN NEW.status NOT IN ('ACTIVE', 'SUPERSEDED', 'INVALIDATED')
BEGIN
  SELECT RAISE(ABORT, 'claims.status must be ACTIVE, SUPERSEDED, or INVALIDATED');
END;

-- ==== legacy 0010_vector_index.sql ====
-- ADR-0017: activate sqlite-vec for vector recall.
--
-- The `vec0` virtual table is the ANN index that backs the vector half of
-- HybridRAG. Dimension is bound to the canonical embedding model
-- (`nomic-embed-text`, 768) per ADR-0017 §"Dimension binding".
--
-- The existing `vector_chunks.embedding_json` column remains the source of
-- truth: this virtual table is rebuildable from it (ADR-0017 §"Recovery").
CREATE VIRTUAL TABLE IF NOT EXISTS vector_index USING vec0(
  chunk_id  INTEGER PRIMARY KEY,
  embedding float[768]
);

-- Backfill: copy any existing chunks whose JSON embedding matches the
-- canonical dimension. Mismatched-dim rows are silently skipped per
-- ADR-0017 §"Populate strategy"; recall falls back to the JSON full-scan
-- for those claims.
INSERT OR IGNORE INTO vector_index(chunk_id, embedding)
SELECT id, embedding_json
  FROM vector_chunks
 WHERE embedding_json IS NOT NULL
   AND json_array_length(embedding_json) = 768;

-- ==== legacy 0011_ontology_enforcement.sql ====
-- ADR-0018: ontology enforcement on claim writes.
--
-- Three additive changes:
--   1. `entities.requires_review` flag for unclassified-entity surfacing.
--   2. Audit columns on `predicate_types` (`created_via`, `created_at`).
--   3. New `predicate_alias` table mapping legacy kebab-case predicates to
--      canonical seeded predicates so historical rows resolve under the
--      new vocabulary.
-- Plus a `BEFORE INSERT` trigger on `claims` that rejects rows whose
-- predicate is neither in `predicate_types.name` nor in
-- `predicate_alias.alias` (defense-in-depth for direct-SQL writes).

ALTER TABLE entities ADD COLUMN requires_review INTEGER NOT NULL DEFAULT 0;

ALTER TABLE predicate_types ADD COLUMN created_via TEXT NOT NULL DEFAULT 'seed';
ALTER TABLE predicate_types ADD COLUMN created_at INTEGER;

CREATE TABLE IF NOT EXISTS predicate_alias (
    alias        TEXT PRIMARY KEY,
    predicate_id INTEGER NOT NULL REFERENCES predicate_types(id) ON DELETE RESTRICT,
    created_at   INTEGER NOT NULL,
    note         TEXT
);

-- Audit log of USER_ASSERTED ontology auto-extensions.
CREATE TABLE IF NOT EXISTS ontology_extension_log (
    id           INTEGER PRIMARY KEY,
    predicate    TEXT NOT NULL,
    parent       TEXT NOT NULL,
    agent_id     TEXT NOT NULL,
    created_at   INTEGER NOT NULL
);

-- M3: extend the seeded ontology to seat the kebab-case predicates that
-- already exist in production (.aver/db.sqlite) under canonical parents.
-- `seed_ontology` will idempotently re-add these on subsequent starts; the
-- explicit INSERT here ensures the rows exist in databases that never run
-- the seed (e.g. raw-SQL replay paths).
INSERT OR IGNORE INTO predicate_types (name, parent_id, created_via, created_at)
VALUES
  ('has_module',    (SELECT id FROM predicate_types WHERE name = 'owns'),       'migration', strftime('%s','now')),
  ('binary_name',   (SELECT id FROM predicate_types WHERE name = 'owns'),       'migration', strftime('%s','now')),
  ('impl_stays_in', (SELECT id FROM predicate_types WHERE name = 'concerns'),   'migration', strftime('%s','now')),
  ('splits_by',     (SELECT id FROM predicate_types WHERE name = 'relates_to'), 'migration', strftime('%s','now')),
  ('uses',          (SELECT id FROM predicate_types WHERE name = 'depends_on'), 'migration', strftime('%s','now'));

-- Aliases for the historical kebab-case forms in log.jsonl so existing rows
-- and replays resolve through the alias table. The trigger and validator
-- accept either the canonical name or any alias.
INSERT OR IGNORE INTO predicate_alias (alias, predicate_id, created_at, note)
SELECT 'has-module', id, strftime('%s','now'), 'M3 alias for log replay'
  FROM predicate_types WHERE name = 'has_module';
INSERT OR IGNORE INTO predicate_alias (alias, predicate_id, created_at, note)
SELECT 'binary-name', id, strftime('%s','now'), 'M3 alias for log replay'
  FROM predicate_types WHERE name = 'binary_name';
INSERT OR IGNORE INTO predicate_alias (alias, predicate_id, created_at, note)
SELECT 'impl-stays-in', id, strftime('%s','now'), 'M3 alias for log replay'
  FROM predicate_types WHERE name = 'impl_stays_in';
INSERT OR IGNORE INTO predicate_alias (alias, predicate_id, created_at, note)
SELECT 'splits-by', id, strftime('%s','now'), 'M3 alias for log replay'
  FROM predicate_types WHERE name = 'splits_by';
-- `uses` is canonical; record the self-alias to make the historical mapping
-- explicit (per ADR-0018 §"Alias-table grammar").
INSERT OR IGNORE INTO predicate_alias (alias, predicate_id, created_at, note)
SELECT 'uses', id, strftime('%s','now'), 'M3 self-alias for log replay'
  FROM predicate_types WHERE name = 'uses';

CREATE TRIGGER IF NOT EXISTS claims_predicate_in_ontology_insert
BEFORE INSERT ON claims
WHEN NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.predicate)
 AND NOT EXISTS (SELECT 1 FROM predicate_alias WHERE alias = NEW.predicate)
BEGIN
  SELECT RAISE(ABORT, 'claims.predicate not in ontology');
END;

CREATE TRIGGER IF NOT EXISTS claims_predicate_in_ontology_update
BEFORE UPDATE OF predicate ON claims
WHEN NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.predicate)
 AND NOT EXISTS (SELECT 1 FROM predicate_alias WHERE alias = NEW.predicate)
BEGIN
  SELECT RAISE(ABORT, 'claims.predicate not in ontology (update)');
END;

-- ==== legacy 0012_source_refs_json_checks.sql ====
-- ADR-0006/ADR-0005: claim source_refs is stored as JSON text but must
-- remain a replayable JSON array. SQLite cannot add a CHECK constraint to
-- an existing table without rebuilding it, so this additive migration uses
-- triggers for defense-in-depth on direct SQL writes.
CREATE TRIGGER IF NOT EXISTS claims_source_refs_json_array_insert
BEFORE INSERT ON claims
WHEN CASE
       WHEN json_valid(NEW.source_refs) = 0 THEN 1
       WHEN json_type(NEW.source_refs) != 'array' THEN 1
       ELSE 0
     END
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs must be a JSON array');
END;

CREATE TRIGGER IF NOT EXISTS claims_source_refs_json_array_update
BEFORE UPDATE OF source_refs ON claims
WHEN CASE
       WHEN json_valid(NEW.source_refs) = 0 THEN 1
       WHEN json_type(NEW.source_refs) != 'array' THEN 1
       ELSE 0
     END
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs must be a JSON array');
END;

-- ==== legacy 0013_observation_source_event_ids_json_checks.sql ====
-- ADR-0016: observation source_event_ids is stored as JSON text and
-- replay/recall code expects a JSON array of event ids. Keep this additive
-- for existing databases by using triggers rather than rebuilding the table.
CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_json_array_insert
BEFORE INSERT ON observations
WHEN CASE
       WHEN json_valid(NEW.source_event_ids) = 0 THEN 1
       WHEN json_type(NEW.source_event_ids) != 'array' THEN 1
       ELSE 0
     END
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids must be a JSON array');
END;

CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_json_array_update
BEFORE UPDATE OF source_event_ids ON observations
WHEN CASE
       WHEN json_valid(NEW.source_event_ids) = 0 THEN 1
       WHEN json_type(NEW.source_event_ids) != 'array' THEN 1
       ELSE 0
     END
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids must be a JSON array');
END;

-- ==== legacy 0014_vector_embedding_json_checks.sql ====
-- ADR-0017: vector_chunks.embedding_json is optional JSON text, but when
-- present it must be an array of numeric embedding components. The dimension
-- is enforced by vector-index population; this trigger guards the JSON shape
-- without rebuilding existing tables.
CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_json_array_insert
BEFORE INSERT ON vector_chunks
WHEN NEW.embedding_json IS NOT NULL
 AND CASE
       WHEN json_valid(NEW.embedding_json) = 0 THEN 1
       WHEN json_type(NEW.embedding_json) != 'array' THEN 1
       ELSE 0
     END
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_json must be NULL or a JSON array');
END;

CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_json_array_update
BEFORE UPDATE OF embedding_json ON vector_chunks
WHEN NEW.embedding_json IS NOT NULL
 AND CASE
       WHEN json_valid(NEW.embedding_json) = 0 THEN 1
       WHEN json_type(NEW.embedding_json) != 'array' THEN 1
       ELSE 0
     END
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_json must be NULL or a JSON array');
END;

-- ==== legacy 0015_vector_embedding_numeric_checks.sql ====
-- ADR-0017: embedding_json arrays must contain numeric components so
-- deterministic vector parsing/rebuild paths cannot be broken by direct SQL
-- rows such as [0.1, "not-a-number"]. Dimension matching remains enforced by
-- vector-index population and recall fallback behavior.
CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_json_numeric_insert
BEFORE INSERT ON vector_chunks
WHEN NEW.embedding_json IS NOT NULL
 AND json_valid(NEW.embedding_json) = 1
 AND json_type(NEW.embedding_json) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.embedding_json)
        WHERE type NOT IN ('integer', 'real')
     )
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_json elements must be numeric');
END;

CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_json_numeric_update
BEFORE UPDATE OF embedding_json ON vector_chunks
WHEN NEW.embedding_json IS NOT NULL
 AND json_valid(NEW.embedding_json) = 1
 AND json_type(NEW.embedding_json) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.embedding_json)
        WHERE type NOT IN ('integer', 'real')
     )
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_json elements must be numeric');
END;

-- ==== legacy 0016_observation_source_event_ids_integer_checks.sql ====
-- ADR-0016: observations.source_event_ids is read as Vec<i64>; direct SQL
-- rows with string/object/null elements would make recall parsing fail later.
-- Keep this as an additive trigger-based invariant for existing databases.
CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_integer_insert
BEFORE INSERT ON observations
WHEN json_valid(NEW.source_event_ids) = 1
 AND json_type(NEW.source_event_ids) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_event_ids)
        WHERE type != 'integer'
     )
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids elements must be integers');
END;

CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_integer_update
BEFORE UPDATE OF source_event_ids ON observations
WHEN json_valid(NEW.source_event_ids) = 1
 AND json_type(NEW.source_event_ids) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_event_ids)
        WHERE type != 'integer'
     )
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids elements must be integers');
END;

-- ==== legacy 0017_claim_source_refs_text_checks.sql ====
-- ADR-0006/ADR-0005: claims.source_refs is parsed as Vec<String>.
-- The JSON array shape is enforced by 0012; this additive migration tightens
-- direct-SQL writes so array elements are text references, not numbers/objects.
CREATE TRIGGER IF NOT EXISTS claims_source_refs_text_insert
BEFORE INSERT ON claims
WHEN json_valid(NEW.source_refs) = 1
 AND json_type(NEW.source_refs) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_refs)
        WHERE type != 'text'
     )
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs elements must be text');
END;

CREATE TRIGGER IF NOT EXISTS claims_source_refs_text_update
BEFORE UPDATE OF source_refs ON claims
WHEN json_valid(NEW.source_refs) = 1
 AND json_type(NEW.source_refs) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_refs)
        WHERE type != 'text'
     )
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs elements must be text');
END;

-- ==== legacy 0018_observation_source_event_ids_nonempty_checks.sql ====
-- ADR-0016: every durable observation must cite at least one source event.
-- Store::record_observation already enforces this; these triggers keep direct
-- SQL writes from creating unsupported observations that recall cannot audit.
CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_nonempty_insert
BEFORE INSERT ON observations
WHEN json_valid(NEW.source_event_ids) = 1
 AND json_type(NEW.source_event_ids) = 'array'
 AND json_array_length(NEW.source_event_ids) = 0
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids must not be empty');
END;

CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_nonempty_update
BEFORE UPDATE OF source_event_ids ON observations
WHEN json_valid(NEW.source_event_ids) = 1
 AND json_type(NEW.source_event_ids) = 'array'
 AND json_array_length(NEW.source_event_ids) = 0
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids must not be empty');
END;

-- ==== legacy 0019_claim_source_refs_nonempty_checks.sql ====
-- ADR-0006/ADR-0005: durable claims must retain at least one provenance
-- reference. Store::add_claim writes one source ref; these triggers prevent
-- direct SQL from stripping claim auditability with an empty JSON array.
CREATE TRIGGER IF NOT EXISTS claims_source_refs_nonempty_insert
BEFORE INSERT ON claims
WHEN json_valid(NEW.source_refs) = 1
 AND json_type(NEW.source_refs) = 'array'
 AND json_array_length(NEW.source_refs) = 0
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs must not be empty');
END;

CREATE TRIGGER IF NOT EXISTS claims_source_refs_nonempty_update
BEFORE UPDATE OF source_refs ON claims
WHEN json_valid(NEW.source_refs) = 1
 AND json_type(NEW.source_refs) = 'array'
 AND json_array_length(NEW.source_refs) = 0
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs must not be empty');
END;

-- ==== legacy 0020_claim_source_refs_nonblank_checks.sql ====
-- ADR-0006/ADR-0005: claim provenance references are meaningful strings.
-- Store::add_claim rejects blank sources; these triggers preserve the same
-- invariant for direct SQL JSON-array updates.
CREATE TRIGGER IF NOT EXISTS claims_source_refs_nonblank_insert
BEFORE INSERT ON claims
WHEN json_valid(NEW.source_refs) = 1
 AND json_type(NEW.source_refs) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_refs)
        WHERE type = 'text' AND trim(value) = ''
     )
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs elements must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS claims_source_refs_nonblank_update
BEFORE UPDATE OF source_refs ON claims
WHEN json_valid(NEW.source_refs) = 1
 AND json_type(NEW.source_refs) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_refs)
        WHERE type = 'text' AND trim(value) = ''
     )
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs elements must not be blank');
END;

-- ==== legacy 0021_observation_source_event_ids_positive_checks.sql ====
-- ADR-0016: observation source_event_ids cite episodic_events.id values,
-- which are positive row ids. Store::record_observation verifies existence;
-- this trigger blocks obviously invalid non-positive ids for direct SQL rows.
CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_positive_insert
BEFORE INSERT ON observations
WHEN json_valid(NEW.source_event_ids) = 1
 AND json_type(NEW.source_event_ids) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_event_ids)
        WHERE type = 'integer' AND value <= 0
     )
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids elements must be positive');
END;

CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_positive_update
BEFORE UPDATE OF source_event_ids ON observations
WHEN json_valid(NEW.source_event_ids) = 1
 AND json_type(NEW.source_event_ids) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_event_ids)
        WHERE type = 'integer' AND value <= 0
     )
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids elements must be positive');
END;

-- ==== legacy 0022_vector_embedding_nonempty_checks.sql ====
-- ADR-0017: when vector_chunks.embedding_json is present, it represents an
-- embedding vector. Store::add_vector_chunk_with_embedding rejects empty
-- vectors; this trigger preserves the same invariant for direct SQL rows.
CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_json_nonempty_insert
BEFORE INSERT ON vector_chunks
WHEN NEW.embedding_json IS NOT NULL
 AND json_valid(NEW.embedding_json) = 1
 AND json_type(NEW.embedding_json) = 'array'
 AND json_array_length(NEW.embedding_json) = 0
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_json must not be empty');
END;

CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_json_nonempty_update
BEFORE UPDATE OF embedding_json ON vector_chunks
WHEN NEW.embedding_json IS NOT NULL
 AND json_valid(NEW.embedding_json) = 1
 AND json_type(NEW.embedding_json) = 'array'
 AND json_array_length(NEW.embedding_json) = 0
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_json must not be empty');
END;

-- ==== legacy 0023_vector_chunk_text_nonblank_checks.sql ====
-- ADR-0006/ADR-0017: vector chunk text is recallable source content.
-- Store::add_vector_chunk validates non-blank text; direct SQL writes should
-- preserve the same invariant so chunk rows remain useful for retrieval.
CREATE TRIGGER IF NOT EXISTS vector_chunks_text_nonblank_insert
BEFORE INSERT ON vector_chunks
WHEN trim(NEW.text) = ''
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.text must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS vector_chunks_text_nonblank_update
BEFORE UPDATE OF text ON vector_chunks
WHEN trim(NEW.text) = ''
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.text must not be blank');
END;

-- ==== legacy 0024_vector_chunk_embedding_model_nonblank_checks.sql ====
-- ADR-0017: vector chunks must record the embedding model that produced the
-- optional embedding. Store writes reject blank model names; direct SQL writes
-- should keep the same replay/debuggability invariant.
CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_model_nonblank_insert
BEFORE INSERT ON vector_chunks
WHEN trim(NEW.embedding_model) = ''
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_model must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS vector_chunks_embedding_model_nonblank_update
BEFORE UPDATE OF embedding_model ON vector_chunks
WHEN trim(NEW.embedding_model) = ''
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.embedding_model must not be blank');
END;

-- ==== legacy 0025_episodic_event_session_nonblank_checks.sql ====
-- ADR-0016: episodic events are grouped by a meaningful session id.
-- Store::record_event rejects blank session ids before logging; direct SQL
-- writes should preserve the same recall/audit invariant.
CREATE TRIGGER IF NOT EXISTS episodic_events_session_id_nonblank_insert
BEFORE INSERT ON episodic_events
WHEN trim(NEW.session_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.session_id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_session_id_nonblank_update
BEFORE UPDATE OF session_id ON episodic_events
WHEN trim(NEW.session_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.session_id must not be blank');
END;

-- ==== legacy 0026_episodic_event_kind_nonblank_checks.sql ====
-- ADR-0016: episodic event kind is the event classifier used by extraction
-- and summaries. Store::record_event rejects blank kinds; direct SQL writes
-- should preserve the same typed event-stream invariant.
CREATE TRIGGER IF NOT EXISTS episodic_events_kind_nonblank_insert
BEFORE INSERT ON episodic_events
WHEN trim(NEW.kind) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.kind must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_kind_nonblank_update
BEFORE UPDATE OF kind ON episodic_events
WHEN trim(NEW.kind) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.kind must not be blank');
END;

-- ==== legacy 0027_episodic_event_source_nonblank_checks.sql ====
-- ADR-0016: episodic events need a source label for auditability.
-- Store::record_event rejects blank sources before logging; direct SQL writes
-- should preserve the same provenance invariant.
CREATE TRIGGER IF NOT EXISTS episodic_events_source_nonblank_insert
BEFORE INSERT ON episodic_events
WHEN trim(NEW.source) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.source must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_source_nonblank_update
BEFORE UPDATE OF source ON episodic_events
WHEN trim(NEW.source) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.source must not be blank');
END;

-- ==== legacy 0028_episodic_event_agent_id_nonblank_checks.sql ====
-- ADR-0016: episodic event agent_id identifies the actor that produced the
-- event. Store::record_event_from_agent validates non-empty agent ids; direct
-- SQL writes should preserve that provenance invariant.
CREATE TRIGGER IF NOT EXISTS episodic_events_agent_id_nonblank_insert
BEFORE INSERT ON episodic_events
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.agent_id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_agent_id_nonblank_update
BEFORE UPDATE OF agent_id ON episodic_events
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.agent_id must not be blank');
END;

-- ==== legacy 0029_episodic_event_agent_id_charset_checks.sql ====
-- ADR-0016: agent ids are portable identifiers used in per-agent audit logs.
-- Rust validation allows only ASCII alphanumeric, '_' and '-'; keep direct SQL
-- rows aligned with that path. Blank ids are handled by the nonblank trigger.
CREATE TRIGGER IF NOT EXISTS episodic_events_agent_id_charset_insert
BEFORE INSERT ON episodic_events
WHEN trim(NEW.agent_id) != ''
 AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.agent_id contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_agent_id_charset_update
BEFORE UPDATE OF agent_id ON episodic_events
WHEN trim(NEW.agent_id) != ''
 AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.agent_id contains invalid characters');
END;

-- ==== legacy 0030_claim_agent_id_nonblank_checks.sql ====
-- ADR-0006: claims carry agent provenance. Store::add_claim validates
-- non-empty agent ids; direct SQL writes should preserve the same audit
-- invariant for durable memory claims.
CREATE TRIGGER IF NOT EXISTS claims_agent_id_nonblank_insert
BEFORE INSERT ON claims
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.agent_id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS claims_agent_id_nonblank_update
BEFORE UPDATE OF agent_id ON claims
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.agent_id must not be blank');
END;

-- ==== legacy 0031_claim_agent_id_charset_checks.sql ====
-- ADR-0006: claim agent ids are portable identifiers used for provenance and
-- per-agent audit logs. Rust validation allows only ASCII alphanumeric, '_'
-- and '-'; keep direct SQL rows aligned with that path.
CREATE TRIGGER IF NOT EXISTS claims_agent_id_charset_insert
BEFORE INSERT ON claims
WHEN trim(NEW.agent_id) != ''
 AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'claims.agent_id contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS claims_agent_id_charset_update
BEFORE UPDATE OF agent_id ON claims
WHEN trim(NEW.agent_id) != ''
 AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'claims.agent_id contains invalid characters');
END;

-- ==== legacy 0032_observation_agent_id_nonblank_checks.sql ====
-- ADR-0016: observations retain the producing agent id for audit/replay.
-- Store::record_observation derives a validated agent id from source events;
-- direct SQL writes should preserve the same provenance invariant.
CREATE TRIGGER IF NOT EXISTS observations_agent_id_nonblank_insert
BEFORE INSERT ON observations
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.agent_id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observations_agent_id_nonblank_update
BEFORE UPDATE OF agent_id ON observations
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.agent_id must not be blank');
END;

-- ==== legacy 0033_observation_agent_id_charset_checks.sql ====
-- ADR-0016: observation agent ids are portable provenance identifiers.
-- Source events are validated by Rust to ASCII alphanumeric, '_' and '-'; keep
-- direct SQL observation rows aligned with that path.
CREATE TRIGGER IF NOT EXISTS observations_agent_id_charset_insert
BEFORE INSERT ON observations
WHEN trim(NEW.agent_id) != ''
 AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'observations.agent_id contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS observations_agent_id_charset_update
BEFORE UPDATE OF agent_id ON observations
WHEN trim(NEW.agent_id) != ''
 AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'observations.agent_id contains invalid characters');
END;

-- ==== legacy 0034_observation_session_nonblank_checks.sql ====
-- ADR-0016: observations are scoped to a concrete session for compaction and
-- replay. Store::record_observation rejects blank session ids; direct SQL rows
-- should preserve the same session provenance invariant.
CREATE TRIGGER IF NOT EXISTS observations_session_id_nonblank_insert
BEFORE INSERT ON observations
WHEN trim(NEW.session_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.session_id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observations_session_id_nonblank_update
BEFORE UPDATE OF session_id ON observations
WHEN trim(NEW.session_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.session_id must not be blank');
END;

-- ==== legacy 0035_observation_content_nonblank_checks.sql ====
-- ADR-0016: observations are source-backed semantic projections. Blank
-- content is not a meaningful projection and Store::record_observation rejects
-- it before logging; direct SQL writes should preserve that invariant.
CREATE TRIGGER IF NOT EXISTS observations_content_nonblank_insert
BEFORE INSERT ON observations
WHEN trim(NEW.content) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.content must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observations_content_nonblank_update
BEFORE UPDATE OF content ON observations
WHEN trim(NEW.content) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.content must not be blank');
END;

-- ==== legacy 0036_observation_derivation_nonblank_checks.sql ====
-- ADR-0016: observations must explain how they were derived from source
-- events. Store::record_observation rejects blank derivations before logging;
-- direct SQL writes should preserve that auditability invariant.
CREATE TRIGGER IF NOT EXISTS observations_derivation_nonblank_insert
BEFORE INSERT ON observations
WHEN trim(NEW.derivation) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.derivation must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observations_derivation_nonblank_update
BEFORE UPDATE OF derivation ON observations
WHEN trim(NEW.derivation) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.derivation must not be blank');
END;

-- ==== legacy 0037_observation_id_nonblank_checks.sql ====
-- Observation ids are deterministic identifiers for source-backed projections.
-- Blank ids cannot be retrieved or audited meaningfully; keep direct SQL rows
-- aligned with Store-generated observation ids.
CREATE TRIGGER IF NOT EXISTS observations_id_nonblank_insert
BEFORE INSERT ON observations
WHEN trim(NEW.id) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observations_id_nonblank_update
BEFORE UPDATE OF id ON observations
WHEN trim(NEW.id) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.id must not be blank');
END;

-- ==== legacy 0038_candidate_claim_subject_nonblank_checks.sql ====
-- Candidate claims are staged triples extracted from source events. The Rust
-- proposal path rejects blank triple fields; direct SQL staging should preserve
-- the same claim-shape invariant.
CREATE TRIGGER IF NOT EXISTS candidate_claims_subject_nonblank_insert
BEFORE INSERT ON candidate_claims
WHEN trim(NEW.subject) = ''
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.subject must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_subject_nonblank_update
BEFORE UPDATE OF subject ON candidate_claims
WHEN trim(NEW.subject) = ''
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.subject must not be blank');
END;

-- ==== legacy 0039_candidate_claim_predicate_nonblank_checks.sql ====
-- Candidate claims are staged triples extracted from source events. The Rust
-- proposal path rejects blank triple fields; direct SQL staging should preserve
-- the same claim-shape invariant.
CREATE TRIGGER IF NOT EXISTS candidate_claims_predicate_nonblank_insert
BEFORE INSERT ON candidate_claims
WHEN trim(NEW.predicate) = ''
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.predicate must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_predicate_nonblank_update
BEFORE UPDATE OF predicate ON candidate_claims
WHEN trim(NEW.predicate) = ''
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.predicate must not be blank');
END;

-- ==== legacy 0040_candidate_claim_object_nonblank_checks.sql ====
-- Candidate claims are staged triples extracted from source events. The Rust
-- proposal path rejects blank triple fields; direct SQL staging should preserve
-- the same claim-shape invariant.
CREATE TRIGGER IF NOT EXISTS candidate_claims_object_nonblank_insert
BEFORE INSERT ON candidate_claims
WHEN trim(NEW.object) = ''
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.object must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_object_nonblank_update
BEFORE UPDATE OF object ON candidate_claims
WHEN trim(NEW.object) = ''
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.object must not be blank');
END;

-- ==== legacy 0041_candidate_rejection_reason_checks.sql ====
-- Rejected candidate claims need an auditable rejection rationale. The Rust
-- Store::reject_candidate_claim path validates nonblank reasons; direct SQL
-- writes should preserve that workflow invariant.
CREATE TRIGGER IF NOT EXISTS candidate_claims_rejection_reason_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.status = 'REJECTED'
 AND (NEW.rejection_reason IS NULL OR trim(NEW.rejection_reason) = '')
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.rejection_reason must not be blank when status is REJECTED');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_rejection_reason_update
BEFORE UPDATE OF status, rejection_reason ON candidate_claims
WHEN NEW.status = 'REJECTED'
 AND (NEW.rejection_reason IS NULL OR trim(NEW.rejection_reason) = '')
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.rejection_reason must not be blank when status is REJECTED');
END;

-- ==== legacy 0042_candidate_promotion_claim_id_checks.sql ====
-- Promoted candidate claims must point at the durable claim created by the
-- promotion path. Store::promote_candidate_claim sets promoted_claim_id and
-- status together; direct SQL writes should preserve that workflow invariant.
CREATE TRIGGER IF NOT EXISTS candidate_claims_promotion_claim_id_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.status = 'PROMOTED' AND NEW.promoted_claim_id IS NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be set when status is PROMOTED');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_promotion_claim_id_update
BEFORE UPDATE OF status, promoted_claim_id ON candidate_claims
WHEN NEW.status = 'PROMOTED' AND NEW.promoted_claim_id IS NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be set when status is PROMOTED');
END;

-- ==== legacy 0043_candidate_created_at_positive_checks.sql ====
-- Candidate claim timestamps are produced from wall-clock Unix seconds by the
-- Store path. Reject sentinel/non-positive timestamps from direct SQL so staged
-- claim ordering and audit views are meaningful.
CREATE TRIGGER IF NOT EXISTS candidate_claims_created_at_positive_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.created_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_created_at_positive_update
BEFORE UPDATE OF created_at ON candidate_claims
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.created_at must be positive');
END;

-- ==== legacy 0044_episodic_event_ts_positive_checks.sql ====
-- Episodic event timestamps come from wall-clock Unix seconds in the Store
-- recording path. Reject non-positive sentinel timestamps from direct SQL so
-- event ordering and extraction thresholds remain meaningful.
CREATE TRIGGER IF NOT EXISTS episodic_events_ts_positive_insert
BEFORE INSERT ON episodic_events
WHEN NEW.ts <= 0
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.ts must be positive');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_ts_positive_update
BEFORE UPDATE OF ts ON episodic_events
WHEN NEW.ts <= 0
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.ts must be positive');
END;

-- ==== legacy 0045_vector_chunk_created_at_positive_checks.sql ====
-- Vector chunk timestamps come from wall-clock Unix seconds in the Store path.
-- Reject non-positive sentinel timestamps from direct SQL so chunk chronology
-- remains meaningful during replay and inspection.
CREATE TRIGGER IF NOT EXISTS vector_chunks_created_at_positive_insert
BEFORE INSERT ON vector_chunks
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.created_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS vector_chunks_created_at_positive_update
BEFORE UPDATE OF created_at ON vector_chunks
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'vector_chunks.created_at must be positive');
END;

-- ==== legacy 0046_observation_ts_positive_checks.sql ====
-- Observation timestamps come from wall-clock Unix seconds in the Store path.
-- Reject non-positive sentinel timestamps from direct SQL so observation
-- chronology and audit projections remain meaningful.
CREATE TRIGGER IF NOT EXISTS observations_ts_positive_insert
BEFORE INSERT ON observations
WHEN NEW.ts <= 0
BEGIN
  SELECT RAISE(ABORT, 'observations.ts must be positive');
END;

CREATE TRIGGER IF NOT EXISTS observations_ts_positive_update
BEFORE UPDATE OF ts ON observations
WHEN NEW.ts <= 0
BEGIN
  SELECT RAISE(ABORT, 'observations.ts must be positive');
END;

-- ==== legacy 0047_candidate_rejection_promotion_exclusion.sql ====
-- Rejected candidates represent discarded staged claims, not promoted durable
-- claims. Store::reject_candidate_claim leaves promoted_claim_id unset; direct
-- SQL writes should preserve the mutually-exclusive workflow states.
CREATE TRIGGER IF NOT EXISTS candidate_claims_rejected_no_promoted_claim_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.status = 'REJECTED' AND NEW.promoted_claim_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be NULL when status is REJECTED');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_rejected_no_promoted_claim_update
BEFORE UPDATE OF status, promoted_claim_id ON candidate_claims
WHEN NEW.status = 'REJECTED' AND NEW.promoted_claim_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be NULL when status is REJECTED');
END;

-- ==== legacy 0048_candidate_promotion_rejection_exclusion.sql ====
-- Promoted candidates point at durable claims, while rejection reasons belong
-- only to rejected candidates. Store::promote_candidate_claim does not set a
-- rejection reason; direct SQL writes should preserve exclusive states.
CREATE TRIGGER IF NOT EXISTS candidate_claims_promoted_no_rejection_reason_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.status = 'PROMOTED' AND NEW.rejection_reason IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.rejection_reason must be NULL when status is PROMOTED');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_promoted_no_rejection_reason_update
BEFORE UPDATE OF status, rejection_reason ON candidate_claims
WHEN NEW.status = 'PROMOTED' AND NEW.rejection_reason IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.rejection_reason must be NULL when status is PROMOTED');
END;

-- ==== legacy 0049_candidate_pending_rejection_exclusion.sql ====
-- Pending candidates have not been accepted or rejected yet, so they should not
-- carry rejection metadata. Store::propose_candidate_claim creates pending rows
-- without a rejection reason; direct SQL writes should preserve that state.
CREATE TRIGGER IF NOT EXISTS candidate_claims_pending_no_rejection_reason_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.status = 'PENDING' AND NEW.rejection_reason IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.rejection_reason must be NULL when status is PENDING');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_pending_no_rejection_reason_update
BEFORE UPDATE OF status, rejection_reason ON candidate_claims
WHEN NEW.status = 'PENDING' AND NEW.rejection_reason IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.rejection_reason must be NULL when status is PENDING');
END;

-- ==== legacy 0050_candidate_pending_promotion_exclusion.sql ====
-- Pending candidates have not yet been promoted, so they should not carry a
-- durable claim reference. Store::propose_candidate_claim creates pending rows
-- without promoted_claim_id; direct SQL writes should preserve that state.
CREATE TRIGGER IF NOT EXISTS candidate_claims_pending_no_promoted_claim_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.status = 'PENDING' AND NEW.promoted_claim_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be NULL when status is PENDING');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_pending_no_promoted_claim_update
BEFORE UPDATE OF status, promoted_claim_id ON candidate_claims
WHEN NEW.status = 'PENDING' AND NEW.promoted_claim_id IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be NULL when status is PENDING');
END;

-- ==== legacy 0051_contradiction_reason_nonblank_checks.sql ====
-- Contradictions are audit records explaining why a claim is challenged.
-- Store::contradict validates nonblank reasons; direct SQL writes should not
-- create unexplained contradiction records.
CREATE TRIGGER IF NOT EXISTS contradictions_reason_nonblank_insert
BEFORE INSERT ON contradictions
WHEN trim(NEW.reason) = ''
BEGIN
  SELECT RAISE(ABORT, 'contradictions.reason must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS contradictions_reason_nonblank_update
BEFORE UPDATE OF reason ON contradictions
WHEN trim(NEW.reason) = ''
BEGIN
  SELECT RAISE(ABORT, 'contradictions.reason must not be blank');
END;

-- ==== legacy 0052_contradiction_created_at_positive_checks.sql ====
-- Contradiction timestamps come from wall-clock Unix seconds in the Store path.
-- Reject non-positive sentinel timestamps from direct SQL so contradiction audit
-- chronology remains meaningful.
CREATE TRIGGER IF NOT EXISTS contradictions_created_at_positive_insert
BEFORE INSERT ON contradictions
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'contradictions.created_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS contradictions_created_at_positive_update
BEFORE UPDATE OF created_at ON contradictions
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'contradictions.created_at must be positive');
END;

-- ==== legacy 0053_contradiction_new_claim_distinct_checks.sql ====
-- A contradiction may cite a replacement/new claim, but that reference must be
-- distinct from the contradicted claim. Self-links add no audit evidence and can
-- confuse contradiction resolution logic.
CREATE TRIGGER IF NOT EXISTS contradictions_new_claim_distinct_insert
BEFORE INSERT ON contradictions
WHEN NEW.new_claim_id IS NOT NULL AND NEW.new_claim_id = NEW.claim_id
BEGIN
  SELECT RAISE(ABORT, 'contradictions.new_claim_id must differ from claim_id');
END;

CREATE TRIGGER IF NOT EXISTS contradictions_new_claim_distinct_update
BEFORE UPDATE OF claim_id, new_claim_id ON contradictions
WHEN NEW.new_claim_id IS NOT NULL AND NEW.new_claim_id = NEW.claim_id
BEGIN
  SELECT RAISE(ABORT, 'contradictions.new_claim_id must differ from claim_id');
END;

-- ==== legacy 0054_entity_type_name_nonblank_checks.sql ====
-- Ontology entity type names are vocabulary identifiers. Reject blank direct
-- SQL inserts/updates so closure rows and classification logic never reference
-- an unnamed type.
CREATE TRIGGER IF NOT EXISTS entity_types_name_nonblank_insert
BEFORE INSERT ON entity_types
WHEN trim(NEW.name) = ''
BEGIN
  SELECT RAISE(ABORT, 'entity_types.name must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS entity_types_name_nonblank_update
BEFORE UPDATE OF name ON entity_types
WHEN trim(NEW.name) = ''
BEGIN
  SELECT RAISE(ABORT, 'entity_types.name must not be blank');
END;

-- ==== legacy 0055_predicate_type_name_nonblank_checks.sql ====
-- Ontology predicate type names are vocabulary identifiers. Reject blank direct
-- SQL inserts/updates so claim predicate validation never accepts unnamed
-- predicate types.
CREATE TRIGGER IF NOT EXISTS predicate_types_name_nonblank_insert
BEFORE INSERT ON predicate_types
WHEN trim(NEW.name) = ''
BEGIN
  SELECT RAISE(ABORT, 'predicate_types.name must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS predicate_types_name_nonblank_update
BEFORE UPDATE OF name ON predicate_types
WHEN trim(NEW.name) = ''
BEGIN
  SELECT RAISE(ABORT, 'predicate_types.name must not be blank');
END;

-- ==== legacy 0056_predicate_alias_nonblank_checks.sql ====
-- Predicate aliases are alternate vocabulary identifiers accepted by claim
-- ontology enforcement. Reject blank aliases so direct SQL cannot create an
-- alias that would make blank predicates appear meaningful.
CREATE TRIGGER IF NOT EXISTS predicate_alias_nonblank_insert
BEFORE INSERT ON predicate_alias
WHEN trim(NEW.alias) = ''
BEGIN
  SELECT RAISE(ABORT, 'predicate_alias.alias must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS predicate_alias_nonblank_update
BEFORE UPDATE OF alias ON predicate_alias
WHEN trim(NEW.alias) = ''
BEGIN
  SELECT RAISE(ABORT, 'predicate_alias.alias must not be blank');
END;

-- ==== legacy 0057_predicate_alias_created_at_positive_checks.sql ====
-- Predicate alias timestamps come from migration/runtime Unix seconds. Reject
-- non-positive sentinel timestamps from direct SQL so ontology alias audit
-- chronology remains meaningful.
CREATE TRIGGER IF NOT EXISTS predicate_alias_created_at_positive_insert
BEFORE INSERT ON predicate_alias
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'predicate_alias.created_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS predicate_alias_created_at_positive_update
BEFORE UPDATE OF created_at ON predicate_alias
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'predicate_alias.created_at must be positive');
END;

-- ==== legacy 0058_ontology_extension_predicate_nonblank_checks.sql ====
-- Ontology extension log entries audit user-asserted vocabulary additions.
-- Reject blank predicates so the audit log always names the extended
-- vocabulary term.
CREATE TRIGGER IF NOT EXISTS ontology_extension_predicate_nonblank_insert
BEFORE INSERT ON ontology_extension_log
WHEN trim(NEW.predicate) = ''
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.predicate must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_predicate_nonblank_update
BEFORE UPDATE OF predicate ON ontology_extension_log
WHEN trim(NEW.predicate) = ''
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.predicate must not be blank');
END;

-- ==== legacy 0059_ontology_extension_parent_nonblank_checks.sql ====
-- Ontology extension log entries must name the parent vocabulary term used to
-- seat a user-asserted predicate. Reject blank parents so audit records explain
-- the extension path.
CREATE TRIGGER IF NOT EXISTS ontology_extension_parent_nonblank_insert
BEFORE INSERT ON ontology_extension_log
WHEN trim(NEW.parent) = ''
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.parent must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_parent_nonblank_update
BEFORE UPDATE OF parent ON ontology_extension_log
WHEN trim(NEW.parent) = ''
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.parent must not be blank');
END;

-- ==== legacy 0060_ontology_extension_agent_id_nonblank_checks.sql ====
-- Ontology extension log entries must name the agent responsible for the
-- user-asserted vocabulary addition. Reject blank agent ids in direct SQL so
-- the extension audit trail remains attributable.
CREATE TRIGGER IF NOT EXISTS ontology_extension_agent_id_nonblank_insert
BEFORE INSERT ON ontology_extension_log
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.agent_id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_agent_id_nonblank_update
BEFORE UPDATE OF agent_id ON ontology_extension_log
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.agent_id must not be blank');
END;

-- ==== legacy 0061_ontology_extension_agent_id_charset_checks.sql ====
-- Keep ontology extension agent ids aligned with Store agent-id validation:
-- nonblank ASCII alphanumeric plus underscore or hyphen only.
CREATE TRIGGER IF NOT EXISTS ontology_extension_agent_id_charset_insert
BEFORE INSERT ON ontology_extension_log
WHEN trim(NEW.agent_id) != '' AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.agent_id contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_agent_id_charset_update
BEFORE UPDATE OF agent_id ON ontology_extension_log
WHEN trim(NEW.agent_id) != '' AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.agent_id contains invalid characters');
END;

-- ==== legacy 0062_ontology_extension_created_at_positive_checks.sql ====
-- Ontology extension timestamps come from runtime/migration Unix seconds.
-- Reject non-positive sentinel timestamps from direct SQL so extension audit
-- chronology remains meaningful.
CREATE TRIGGER IF NOT EXISTS ontology_extension_created_at_positive_insert
BEFORE INSERT ON ontology_extension_log
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.created_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_created_at_positive_update
BEFORE UPDATE OF created_at ON ontology_extension_log
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.created_at must be positive');
END;

-- ==== legacy 0063_ontology_extension_parent_exists_checks.sql ====
-- Ontology extension log parent values should refer to seeded or extended
-- predicate vocabulary terms. This keeps audit rows tied to a real ontology
-- seating point even for direct SQL writes.
CREATE TRIGGER IF NOT EXISTS ontology_extension_parent_exists_insert
BEFORE INSERT ON ontology_extension_log
WHEN trim(NEW.parent) != ''
 AND NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.parent)
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.parent must exist in predicate_types');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_parent_exists_update
BEFORE UPDATE OF parent ON ontology_extension_log
WHEN trim(NEW.parent) != ''
 AND NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.parent)
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.parent must exist in predicate_types');
END;

-- ==== legacy 0064_entity_type_parent_distinct_checks.sql ====
-- Entity type hierarchy edges should not be self-loops. Direct SQL can create
-- a row whose parent_id equals its id, which makes closure/hierarchy semantics
-- ambiguous.
CREATE TRIGGER IF NOT EXISTS entity_types_parent_distinct_insert
BEFORE INSERT ON entity_types
WHEN NEW.parent_id IS NOT NULL AND NEW.parent_id = NEW.id
BEGIN
  SELECT RAISE(ABORT, 'entity_types.parent_id must differ from id');
END;

CREATE TRIGGER IF NOT EXISTS entity_types_parent_distinct_update
BEFORE UPDATE OF id, parent_id ON entity_types
WHEN NEW.parent_id IS NOT NULL AND NEW.parent_id = NEW.id
BEGIN
  SELECT RAISE(ABORT, 'entity_types.parent_id must differ from id');
END;

-- ==== legacy 0065_predicate_type_parent_distinct_checks.sql ====
-- Predicate type hierarchy edges should not be self-loops. Direct SQL can
-- create a row whose parent_id equals its id, which makes closure/hierarchy
-- semantics ambiguous.
CREATE TRIGGER IF NOT EXISTS predicate_types_parent_distinct_insert
BEFORE INSERT ON predicate_types
WHEN NEW.parent_id IS NOT NULL AND NEW.parent_id = NEW.id
BEGIN
  SELECT RAISE(ABORT, 'predicate_types.parent_id must differ from id');
END;

CREATE TRIGGER IF NOT EXISTS predicate_types_parent_distinct_update
BEFORE UPDATE OF id, parent_id ON predicate_types
WHEN NEW.parent_id IS NOT NULL AND NEW.parent_id = NEW.id
BEGIN
  SELECT RAISE(ABORT, 'predicate_types.parent_id must differ from id');
END;

-- ==== legacy 0066_entity_name_nonblank_checks.sql ====
-- Projected graph entity names come from claim subjects/objects and should be
-- meaningful identifiers. Reject blank direct SQL rows so entity classification
-- and graph expansion never reference unnamed entities.
CREATE TRIGGER IF NOT EXISTS entities_name_nonblank_insert
BEFORE INSERT ON entities
WHEN trim(NEW.name) = ''
BEGIN
  SELECT RAISE(ABORT, 'entities.name must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS entities_name_nonblank_update
BEFORE UPDATE OF name ON entities
WHEN trim(NEW.name) = ''
BEGIN
  SELECT RAISE(ABORT, 'entities.name must not be blank');
END;

-- ==== legacy 0067_entity_created_at_positive_checks.sql ====
-- Projected graph entity timestamps come from Store write/replay paths. Reject
-- non-positive sentinel creation timestamps from direct SQL so entity audit
-- chronology remains meaningful.
CREATE TRIGGER IF NOT EXISTS entities_created_at_positive_insert
BEFORE INSERT ON entities
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'entities.created_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS entities_created_at_positive_update
BEFORE UPDATE OF created_at ON entities
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'entities.created_at must be positive');
END;

-- ==== legacy 0068_entity_last_seen_at_positive_checks.sql ====
-- Projected graph entity last-seen timestamps come from Store write/replay
-- paths. Reject non-positive sentinel timestamps from direct SQL so entity
-- recency and decay logic remains meaningful.
CREATE TRIGGER IF NOT EXISTS entities_last_seen_at_positive_insert
BEFORE INSERT ON entities
WHEN NEW.last_seen_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'entities.last_seen_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS entities_last_seen_at_positive_update
BEFORE UPDATE OF last_seen_at ON entities
WHEN NEW.last_seen_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'entities.last_seen_at must be positive');
END;

-- ==== legacy 0069_entity_seen_after_created_checks.sql ====
-- Projected entity recency cannot precede creation. Enforce this for direct
-- SQL writes so graph recency, decay, and audit chronology stay coherent.
CREATE TRIGGER IF NOT EXISTS entities_seen_after_created_insert
BEFORE INSERT ON entities
WHEN NEW.created_at > 0 AND NEW.last_seen_at > 0 AND NEW.last_seen_at < NEW.created_at
BEGIN
  SELECT RAISE(ABORT, 'entities.last_seen_at must be >= created_at');
END;

CREATE TRIGGER IF NOT EXISTS entities_seen_after_created_update
BEFORE UPDATE OF created_at, last_seen_at ON entities
WHEN NEW.created_at > 0 AND NEW.last_seen_at > 0 AND NEW.last_seen_at < NEW.created_at
BEGIN
  SELECT RAISE(ABORT, 'entities.last_seen_at must be >= created_at');
END;

-- ==== legacy 0070_privacy_rejection_reason_nonblank_checks.sql ====
-- Privacy rejection telemetry stores aggregate reason labels only. Reject blank
-- direct SQL rows so counters remain actionable without containing secret data.
CREATE TRIGGER IF NOT EXISTS privacy_rejections_reason_nonblank_insert
BEFORE INSERT ON privacy_rejections
WHEN trim(NEW.reason) = ''
BEGIN
  SELECT RAISE(ABORT, 'privacy_rejections.reason must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS privacy_rejections_reason_nonblank_update
BEFORE UPDATE OF reason ON privacy_rejections
WHEN trim(NEW.reason) = ''
BEGIN
  SELECT RAISE(ABORT, 'privacy_rejections.reason must not be blank');
END;

-- ==== legacy 0071_privacy_rejection_count_positive_checks.sql ====
-- Privacy rejection telemetry counts aggregate observed rejections. Reject
-- non-positive direct SQL rows so telemetry snapshots stay meaningful.
CREATE TRIGGER IF NOT EXISTS privacy_rejections_count_positive_insert
BEFORE INSERT ON privacy_rejections
WHEN NEW.count <= 0
BEGIN
  SELECT RAISE(ABORT, 'privacy_rejections.count must be positive');
END;

CREATE TRIGGER IF NOT EXISTS privacy_rejections_count_positive_update
BEFORE UPDATE OF count ON privacy_rejections
WHEN NEW.count <= 0
BEGIN
  SELECT RAISE(ABORT, 'privacy_rejections.count must be positive');
END;

-- ==== legacy 0072_ontology_extension_predicate_exists_checks.sql ====
-- Ontology extension log predicate values should refer to predicate vocabulary
-- terms that actually exist. This keeps audit rows tied to the row added by
-- the extension path even for direct SQL writes.
CREATE TRIGGER IF NOT EXISTS ontology_extension_predicate_exists_insert
BEFORE INSERT ON ontology_extension_log
WHEN trim(NEW.predicate) != ''
 AND NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.predicate)
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.predicate must exist in predicate_types');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_predicate_exists_update
BEFORE UPDATE OF predicate ON ontology_extension_log
WHEN trim(NEW.predicate) != ''
 AND NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.predicate)
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.predicate must exist in predicate_types');
END;

-- ==== legacy 0073_claim_subject_nonblank_checks.sql ====
-- Durable claim triples must keep nonblank fields even for direct SQL updates.
-- Store::add_claim validates subject/predicate/object before log-first writes;
-- mirror the subject invariant at the schema boundary.
CREATE TRIGGER IF NOT EXISTS claims_subject_nonblank_insert
BEFORE INSERT ON claims
WHEN trim(NEW.subject) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.subject must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS claims_subject_nonblank_update
BEFORE UPDATE OF subject ON claims
WHEN trim(NEW.subject) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.subject must not be blank');
END;

-- ==== legacy 0074_claim_object_nonblank_checks.sql ====
-- Durable claim triples must keep nonblank fields even for direct SQL updates.
-- Store::add_claim validates subject/predicate/object before log-first writes;
-- mirror the object invariant at the schema boundary.
CREATE TRIGGER IF NOT EXISTS claims_object_nonblank_insert
BEFORE INSERT ON claims
WHEN trim(NEW.object) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.object must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS claims_object_nonblank_update
BEFORE UPDATE OF object ON claims
WHEN trim(NEW.object) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.object must not be blank');
END;

-- ==== legacy 0075_claim_created_at_positive_checks.sql ====
-- Durable claim timestamps are written from the Store clock. Reject sentinel or
-- non-positive creation times from direct SQL so claim chronology remains sane.
CREATE TRIGGER IF NOT EXISTS claims_created_at_positive_insert
BEFORE INSERT ON claims
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.created_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS claims_created_at_positive_update
BEFORE UPDATE OF created_at ON claims
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.created_at must be positive');
END;

-- ==== legacy 0076_claim_last_seen_at_positive_checks.sql ====
-- Durable claim timestamps are written from the Store clock. Reject sentinel or
-- non-positive last-seen times from direct SQL so claim chronology remains sane.
CREATE TRIGGER IF NOT EXISTS claims_last_seen_at_positive_insert
BEFORE INSERT ON claims
WHEN NEW.last_seen_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.last_seen_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS claims_last_seen_at_positive_update
BEFORE UPDATE OF last_seen_at ON claims
WHEN NEW.last_seen_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.last_seen_at must be positive');
END;

-- ==== legacy 0077_claim_seen_after_created_checks.sql ====
-- Durable claim chronology should not move backwards. Guard with positivity so
-- earlier timestamp-specific triggers produce their more precise errors first.
CREATE TRIGGER IF NOT EXISTS claims_seen_after_created_insert
BEFORE INSERT ON claims
WHEN NEW.created_at > 0
 AND NEW.last_seen_at > 0
 AND NEW.last_seen_at < NEW.created_at
BEGIN
  SELECT RAISE(ABORT, 'claims.last_seen_at must be >= created_at');
END;

CREATE TRIGGER IF NOT EXISTS claims_seen_after_created_update
BEFORE UPDATE OF created_at, last_seen_at ON claims
WHEN NEW.created_at > 0
 AND NEW.last_seen_at > 0
 AND NEW.last_seen_at < NEW.created_at
BEGIN
  SELECT RAISE(ABORT, 'claims.last_seen_at must be >= created_at');
END;

-- ==== legacy 0078_claim_write_ts_positive_checks.sql ====
-- Durable claims are appended to the JSONL log before SQLite projection; keep the
-- projected write timestamp positive so replay/audit ordering cannot use zero.
CREATE TRIGGER IF NOT EXISTS claims_write_ts_positive_insert
BEFORE INSERT ON claims
WHEN NEW.write_ts <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.write_ts must be positive');
END;

CREATE TRIGGER IF NOT EXISTS claims_write_ts_positive_update
BEFORE UPDATE OF write_ts ON claims
WHEN NEW.write_ts <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.write_ts must be positive');
END;

-- ==== legacy 0079_claim_last_verified_at_positive_checks.sql ====
-- `last_verified_at` is optional, but when present it is written from the Store
-- clock for policy scoring and should not use sentinel or non-positive values.
CREATE TRIGGER IF NOT EXISTS claims_last_verified_at_positive_insert
BEFORE INSERT ON claims
WHEN NEW.last_verified_at IS NOT NULL
 AND NEW.last_verified_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.last_verified_at must be positive when set');
END;

CREATE TRIGGER IF NOT EXISTS claims_last_verified_at_positive_update
BEFORE UPDATE OF last_verified_at ON claims
WHEN NEW.last_verified_at IS NOT NULL
 AND NEW.last_verified_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'claims.last_verified_at must be positive when set');
END;

-- ==== legacy 0080_claim_verified_after_created_checks.sql ====
-- Optional verification timestamps should not predate the claim creation time.
-- Guard with positivity so timestamp-specific triggers emit precise errors first.
CREATE TRIGGER IF NOT EXISTS claims_verified_after_created_insert
BEFORE INSERT ON claims
WHEN NEW.created_at > 0
 AND NEW.last_verified_at IS NOT NULL
 AND NEW.last_verified_at > 0
 AND NEW.last_verified_at < NEW.created_at
BEGIN
  SELECT RAISE(ABORT, 'claims.last_verified_at must be >= created_at when set');
END;

CREATE TRIGGER IF NOT EXISTS claims_verified_after_created_update
BEFORE UPDATE OF created_at, last_verified_at ON claims
WHEN NEW.created_at > 0
 AND NEW.last_verified_at IS NOT NULL
 AND NEW.last_verified_at > 0
 AND NEW.last_verified_at < NEW.created_at
BEGIN
  SELECT RAISE(ABORT, 'claims.last_verified_at must be >= created_at when set');
END;

-- ==== legacy 0081_claim_predicate_nonblank_checks.sql ====
-- Durable claim predicates are ontology vocabulary identifiers. Reject blank
-- direct SQL writes with a precise error before ontology membership checks.
CREATE TRIGGER IF NOT EXISTS claims_predicate_nonblank_insert
BEFORE INSERT ON claims
WHEN trim(NEW.predicate) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.predicate must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS claims_predicate_nonblank_update
BEFORE UPDATE OF predicate ON claims
WHEN trim(NEW.predicate) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.predicate must not be blank');
END;

-- ==== legacy 0082_claim_created_after_write_checks.sql ====
-- A durable claim is appended to the audit log before its SQLite projection is
-- created. The projected creation timestamp must not predate the write time.
CREATE TRIGGER IF NOT EXISTS claims_created_after_write_insert
BEFORE INSERT ON claims
WHEN NEW.write_ts > 0
 AND NEW.created_at > 0
 AND NEW.created_at < NEW.write_ts
BEGIN
  SELECT RAISE(ABORT, 'claims.created_at must be >= write_ts');
END;

CREATE TRIGGER IF NOT EXISTS claims_created_after_write_update
BEFORE UPDATE OF write_ts, created_at ON claims
WHEN NEW.write_ts > 0
 AND NEW.created_at > 0
 AND NEW.created_at < NEW.write_ts
BEGIN
  SELECT RAISE(ABORT, 'claims.created_at must be >= write_ts');
END;

-- ==== legacy 0083_observation_prune_markers.sql ====
CREATE TABLE IF NOT EXISTS observation_prune_markers (
  id                    TEXT PRIMARY KEY,
  session_id            TEXT    NOT NULL,
  pruned_observation_ids TEXT   NOT NULL,
  ts                    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS observation_prune_markers_session_ts
  ON observation_prune_markers(session_id, ts);

-- ==== legacy 0084_hyperedges.sql ====
CREATE TABLE IF NOT EXISTS hyperedges (
  id          INTEGER PRIMARY KEY,
  predicate   TEXT    NOT NULL,
  provenance  TEXT    NOT NULL CHECK (provenance IN ('USER_ASSERTED','EXTRACTED','INFERRED','AMBIGUOUS')),
  confidence  REAL    NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
  source_refs TEXT    NOT NULL CHECK (json_valid(source_refs) AND json_type(source_refs) = 'array'),
  status      TEXT    NOT NULL DEFAULT 'ACTIVE' CHECK (status IN ('ACTIVE','SUPERSEDED','INVALIDATED')),
  created_at  INTEGER NOT NULL CHECK (created_at > 0),
  updated_at  INTEGER NOT NULL CHECK (updated_at > 0)
);

CREATE INDEX IF NOT EXISTS hyperedges_status_predicate
  ON hyperedges(status, predicate);

CREATE TABLE IF NOT EXISTS hyperedge_participants (
  id           INTEGER PRIMARY KEY,
  hyperedge_id INTEGER NOT NULL REFERENCES hyperedges(id) ON DELETE CASCADE,
  role         TEXT    NOT NULL CHECK (trim(role) <> ''),
  entity       TEXT    NOT NULL CHECK (trim(entity) <> '')
);

CREATE INDEX IF NOT EXISTS hyperedge_participants_entity
  ON hyperedge_participants(entity);

CREATE INDEX IF NOT EXISTS hyperedge_participants_hyperedge_id
  ON hyperedge_participants(hyperedge_id);

-- ==== legacy 0085_scope_column.sql ====
-- ADR-0021: scope as a first-class memory dimension.
--
-- Add `scope TEXT NOT NULL DEFAULT 'global'` to the four memory-bearing
-- tables (claims, episodic_events, observations, candidate_claims), with
-- per-table indexes and BEFORE INSERT/UPDATE triggers that enforce the
-- charset `[A-Za-z0-9_/-]` and reject blanks.
--
-- Migration is purely additive. Existing rows default to 'global', which
-- is the correct scope for the two ACTIVE claims observed at the time of
-- ADR-0021 (vasudev-core / is_about and user / prefers_code_review_model).

ALTER TABLE claims            ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';
ALTER TABLE episodic_events   ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';
ALTER TABLE observations      ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';
ALTER TABLE candidate_claims  ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';

CREATE INDEX IF NOT EXISTS claims_scope            ON claims(scope);
CREATE INDEX IF NOT EXISTS episodic_events_scope   ON episodic_events(scope);
CREATE INDEX IF NOT EXISTS observations_scope      ON observations(scope);
CREATE INDEX IF NOT EXISTS candidate_claims_scope  ON candidate_claims(scope);

-- Charset trigger pattern mirrors `claims_agent_id_charset_*` from
-- migration 0031: GLOB negation across the allowed character class.
-- Allowed: A-Z a-z 0-9 underscore hyphen forward-slash.

CREATE TRIGGER IF NOT EXISTS claims_scope_nonblank_insert
BEFORE INSERT ON claims
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS claims_scope_nonblank_update
BEFORE UPDATE OF scope ON claims
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS claims_scope_charset_insert
BEFORE INSERT ON claims
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'claims.scope contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS claims_scope_charset_update
BEFORE UPDATE OF scope ON claims
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'claims.scope contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_scope_nonblank_insert
BEFORE INSERT ON episodic_events
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_scope_nonblank_update
BEFORE UPDATE OF scope ON episodic_events
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_scope_charset_insert
BEFORE INSERT ON episodic_events
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.scope contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_scope_charset_update
BEFORE UPDATE OF scope ON episodic_events
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.scope contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS observations_scope_nonblank_insert
BEFORE INSERT ON observations
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observations_scope_nonblank_update
BEFORE UPDATE OF scope ON observations
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observations_scope_charset_insert
BEFORE INSERT ON observations
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'observations.scope contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS observations_scope_charset_update
BEFORE UPDATE OF scope ON observations
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'observations.scope contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_scope_nonblank_insert
BEFORE INSERT ON candidate_claims
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_scope_nonblank_update
BEFORE UPDATE OF scope ON candidate_claims
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_scope_charset_insert
BEFORE INSERT ON candidate_claims
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.scope contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_scope_charset_update
BEFORE UPDATE OF scope ON candidate_claims
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.scope contains invalid characters');
END;

-- ==== legacy 0086_requires_predicate_alias.sql ====
-- Add a semantic predicate alias for model-authored dependency claims.
-- `requires` means the same relationship as canonical `depends_on`, but
-- keeping it as an alias preserves source/log fidelity for callers that emit
-- natural-language relation names.
--
-- `depends_on` is normally seeded by seed_ontology(), which runs after all
-- migrations. We INSERT OR IGNORE it here so the alias foreign-key resolves
-- on fresh databases where the seed has not yet run.
INSERT OR IGNORE INTO predicate_types (name, parent_id, created_via, created_at)
VALUES ('depends_on', NULL, 'migration', strftime('%s','now'));

INSERT OR IGNORE INTO predicate_alias (alias, predicate_id, created_at, note)
SELECT 'requires', id, strftime('%s','now'), 'semantic alias for depends_on'
  FROM predicate_types
 WHERE name = 'depends_on';

-- ==== legacy 0087_hyperedge_source_refs_nonempty_checks.sql ====
CREATE TRIGGER IF NOT EXISTS hyperedges_source_refs_nonempty_insert
BEFORE INSERT ON hyperedges
WHEN json_valid(NEW.source_refs) AND json_type(NEW.source_refs) = 'array' AND json_array_length(NEW.source_refs) = 0
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.source_refs must not be empty');
END;

CREATE TRIGGER IF NOT EXISTS hyperedges_source_refs_nonempty_update
BEFORE UPDATE OF source_refs ON hyperedges
WHEN json_valid(NEW.source_refs) AND json_type(NEW.source_refs) = 'array' AND json_array_length(NEW.source_refs) = 0
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.source_refs must not be empty');
END;

-- ==== legacy 0088_hyperedge_predicate_nonblank_checks.sql ====
CREATE TRIGGER IF NOT EXISTS hyperedges_predicate_nonblank_insert
BEFORE INSERT ON hyperedges
WHEN trim(NEW.predicate) = ''
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.predicate must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS hyperedges_predicate_nonblank_update
BEFORE UPDATE OF predicate ON hyperedges
WHEN trim(NEW.predicate) = ''
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.predicate must not be blank');
END;

-- ==== legacy 0089_hyperedge_updated_after_created_checks.sql ====
CREATE TRIGGER IF NOT EXISTS hyperedges_updated_after_created_insert
BEFORE INSERT ON hyperedges
WHEN NEW.updated_at < NEW.created_at
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.updated_at must be >= created_at');
END;

CREATE TRIGGER IF NOT EXISTS hyperedges_updated_after_created_update
BEFORE UPDATE OF created_at, updated_at ON hyperedges
WHEN NEW.updated_at < NEW.created_at
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.updated_at must be >= created_at');
END;

-- ==== legacy 0090_hyperedge_participant_hyperedge_id_positive_checks.sql ====
CREATE TRIGGER IF NOT EXISTS hyperedge_participants_hyperedge_id_positive_insert
BEFORE INSERT ON hyperedge_participants
WHEN NEW.hyperedge_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'hyperedge_participants.hyperedge_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS hyperedge_participants_hyperedge_id_positive_update
BEFORE UPDATE OF hyperedge_id ON hyperedge_participants
WHEN NEW.hyperedge_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'hyperedge_participants.hyperedge_id must be positive');
END;

-- ==== legacy 0091_observation_prune_marker_ts_positive_checks.sql ====
CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ts_positive_insert
BEFORE INSERT ON observation_prune_markers
WHEN NEW.ts <= 0
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.ts must be positive');
END;

CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ts_positive_update
BEFORE UPDATE OF ts ON observation_prune_markers
WHEN NEW.ts <= 0
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.ts must be positive');
END;

-- ==== legacy 0092_observation_prune_marker_ids_json_array_checks.sql ====
CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_json_array_insert
BEFORE INSERT ON observation_prune_markers
WHEN NOT (json_valid(NEW.pruned_observation_ids) AND json_type(NEW.pruned_observation_ids) = 'array')
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids must be a JSON array');
END;

CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_json_array_update
BEFORE UPDATE OF pruned_observation_ids ON observation_prune_markers
WHEN NOT (json_valid(NEW.pruned_observation_ids) AND json_type(NEW.pruned_observation_ids) = 'array')
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids must be a JSON array');
END;

-- ==== legacy 0093_observation_prune_marker_ids_nonempty_checks.sql ====
CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_nonempty_insert
BEFORE INSERT ON observation_prune_markers
WHEN json_valid(NEW.pruned_observation_ids)
    AND json_type(NEW.pruned_observation_ids) = 'array'
    AND json_array_length(NEW.pruned_observation_ids) = 0
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids must not be empty');
END;

CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_nonempty_update
BEFORE UPDATE OF pruned_observation_ids ON observation_prune_markers
WHEN json_valid(NEW.pruned_observation_ids)
    AND json_type(NEW.pruned_observation_ids) = 'array'
    AND json_array_length(NEW.pruned_observation_ids) = 0
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids must not be empty');
END;

-- ==== legacy 0094_observation_prune_marker_session_id_nonblank_checks.sql ====
CREATE TRIGGER IF NOT EXISTS observation_prune_markers_session_id_nonblank_insert
BEFORE INSERT ON observation_prune_markers
WHEN trim(NEW.session_id) = ''
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.session_id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observation_prune_markers_session_id_nonblank_update
BEFORE UPDATE OF session_id ON observation_prune_markers
WHEN trim(NEW.session_id) = ''
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.session_id must not be blank');
END;

-- ==== legacy 0095_observation_prune_marker_id_nonblank_checks.sql ====
CREATE TRIGGER IF NOT EXISTS observation_prune_markers_id_nonblank_insert
BEFORE INSERT ON observation_prune_markers
WHEN trim(NEW.id) = ''
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observation_prune_markers_id_nonblank_update
BEFORE UPDATE OF id ON observation_prune_markers
WHEN trim(NEW.id) = ''
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.id must not be blank');
END;

-- ==== legacy 0096_hyperedge_source_refs_elements_nonblank_checks.sql ====
CREATE TRIGGER IF NOT EXISTS hyperedges_source_refs_elements_nonblank_insert
BEFORE INSERT ON hyperedges
WHEN json_valid(NEW.source_refs)
    AND json_type(NEW.source_refs) = 'array'
    AND EXISTS (
        SELECT 1
        FROM json_each(NEW.source_refs)
        WHERE json_each.type != 'text' OR trim(json_each.value) = ''
    )
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.source_refs elements must be nonblank strings');
END;

CREATE TRIGGER IF NOT EXISTS hyperedges_source_refs_elements_nonblank_update
BEFORE UPDATE OF source_refs ON hyperedges
WHEN json_valid(NEW.source_refs)
    AND json_type(NEW.source_refs) = 'array'
    AND EXISTS (
        SELECT 1
        FROM json_each(NEW.source_refs)
        WHERE json_each.type != 'text' OR trim(json_each.value) = ''
    )
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.source_refs elements must be nonblank strings');
END;

-- ==== legacy 0097_observation_prune_marker_ids_elements_nonblank_checks.sql ====
CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_elements_nonblank_insert
BEFORE INSERT ON observation_prune_markers
WHEN json_valid(NEW.pruned_observation_ids)
    AND json_type(NEW.pruned_observation_ids) = 'array'
    AND EXISTS (
        SELECT 1
        FROM json_each(NEW.pruned_observation_ids)
        WHERE json_each.type != 'text' OR trim(json_each.value) = ''
    )
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids elements must be nonblank strings');
END;

CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_elements_nonblank_update
BEFORE UPDATE OF pruned_observation_ids ON observation_prune_markers
WHEN json_valid(NEW.pruned_observation_ids)
    AND json_type(NEW.pruned_observation_ids) = 'array'
    AND EXISTS (
        SELECT 1
        FROM json_each(NEW.pruned_observation_ids)
        WHERE json_each.type != 'text' OR trim(json_each.value) = ''
    )
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids elements must be nonblank strings');
END;

-- ==== legacy 0098_hyperedge_source_refs_unique_checks.sql ====
CREATE TRIGGER IF NOT EXISTS hyperedges_source_refs_unique_insert
BEFORE INSERT ON hyperedges
WHEN json_valid(NEW.source_refs)
    AND json_type(NEW.source_refs) = 'array'
    AND (
        SELECT COUNT(*) != COUNT(DISTINCT json_each.value)
        FROM json_each(NEW.source_refs)
        WHERE json_each.type = 'text'
    )
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.source_refs elements must be unique');
END;

CREATE TRIGGER IF NOT EXISTS hyperedges_source_refs_unique_update
BEFORE UPDATE OF source_refs ON hyperedges
WHEN json_valid(NEW.source_refs)
    AND json_type(NEW.source_refs) = 'array'
    AND (
        SELECT COUNT(*) != COUNT(DISTINCT json_each.value)
        FROM json_each(NEW.source_refs)
        WHERE json_each.type = 'text'
    )
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.source_refs elements must be unique');
END;

-- ==== legacy 0099_observation_prune_marker_ids_unique_checks.sql ====
CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_unique_insert
BEFORE INSERT ON observation_prune_markers
WHEN json_valid(NEW.pruned_observation_ids)
    AND json_type(NEW.pruned_observation_ids) = 'array'
    AND (
        SELECT COUNT(*) != COUNT(DISTINCT json_each.value)
        FROM json_each(NEW.pruned_observation_ids)
        WHERE json_each.type = 'text'
    )
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids elements must be unique');
END;

CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_unique_update
BEFORE UPDATE OF pruned_observation_ids ON observation_prune_markers
WHEN json_valid(NEW.pruned_observation_ids)
    AND json_type(NEW.pruned_observation_ids) = 'array'
    AND (
        SELECT COUNT(*) != COUNT(DISTINCT json_each.value)
        FROM json_each(NEW.pruned_observation_ids)
        WHERE json_each.type = 'text'
    )
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids elements must be unique');
END;

-- ==== legacy 0100_hyperedge_participants_unique_role_entity.sql ====
CREATE UNIQUE INDEX IF NOT EXISTS hyperedge_participants_hyperedge_role_entity
ON hyperedge_participants(hyperedge_id, role, entity);

CREATE TRIGGER IF NOT EXISTS hyperedge_participants_duplicate_role_entity_insert
BEFORE INSERT ON hyperedge_participants
WHEN EXISTS (
    SELECT 1
    FROM hyperedge_participants existing
    WHERE existing.hyperedge_id = NEW.hyperedge_id
      AND existing.role = NEW.role
      AND existing.entity = NEW.entity
)
BEGIN
    SELECT RAISE(ABORT, 'hyperedge_participants role/entity must be unique per hyperedge');
END;

CREATE TRIGGER IF NOT EXISTS hyperedge_participants_duplicate_role_entity_update
BEFORE UPDATE OF hyperedge_id, role, entity ON hyperedge_participants
WHEN EXISTS (
    SELECT 1
    FROM hyperedge_participants existing
    WHERE existing.id != NEW.id
      AND existing.hyperedge_id = NEW.hyperedge_id
      AND existing.role = NEW.role
      AND existing.entity = NEW.entity
)
BEGIN
    SELECT RAISE(ABORT, 'hyperedge_participants role/entity must be unique per hyperedge');
END;

-- ==== legacy 0101_hyperedge_predicate_type_checks.sql ====
CREATE TRIGGER IF NOT EXISTS hyperedges_predicate_type_insert
BEFORE INSERT ON hyperedges
WHEN trim(NEW.predicate) != ''
    AND NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.predicate)
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.predicate not in predicate_types');
END;

CREATE TRIGGER IF NOT EXISTS hyperedges_predicate_type_update
BEFORE UPDATE OF predicate ON hyperedges
WHEN trim(NEW.predicate) != ''
    AND NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.predicate)
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.predicate not in predicate_types');
END;

-- ==== legacy 0102_claim_source_refs_unique_checks.sql ====
CREATE TRIGGER IF NOT EXISTS claims_source_refs_unique_insert
BEFORE INSERT ON claims
WHEN json_valid(NEW.source_refs)
    AND json_type(NEW.source_refs) = 'array'
    AND (
        SELECT COUNT(*) != COUNT(DISTINCT json_each.value)
        FROM json_each(NEW.source_refs)
        WHERE json_each.type = 'text'
    )
BEGIN
    SELECT RAISE(ABORT, 'claims.source_refs elements must be unique');
END;

CREATE TRIGGER IF NOT EXISTS claims_source_refs_unique_update
BEFORE UPDATE OF source_refs ON claims
WHEN json_valid(NEW.source_refs)
    AND json_type(NEW.source_refs) = 'array'
    AND (
        SELECT COUNT(*) != COUNT(DISTINCT json_each.value)
        FROM json_each(NEW.source_refs)
        WHERE json_each.type = 'text'
    )
BEGIN
    SELECT RAISE(ABORT, 'claims.source_refs elements must be unique');
END;

-- ==== legacy 0103_observation_source_event_ids_unique_checks.sql ====
CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_unique_insert
BEFORE INSERT ON observations
WHEN json_valid(NEW.source_event_ids)
    AND json_type(NEW.source_event_ids) = 'array'
    AND (
        SELECT COUNT(*) != COUNT(DISTINCT json_each.value)
        FROM json_each(NEW.source_event_ids)
        WHERE json_each.type = 'integer'
    )
BEGIN
    SELECT RAISE(ABORT, 'observations.source_event_ids elements must be unique');
END;

CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_unique_update
BEFORE UPDATE OF source_event_ids ON observations
WHEN json_valid(NEW.source_event_ids)
    AND json_type(NEW.source_event_ids) = 'array'
    AND (
        SELECT COUNT(*) != COUNT(DISTINCT json_each.value)
        FROM json_each(NEW.source_event_ids)
        WHERE json_each.type = 'integer'
    )
BEGIN
    SELECT RAISE(ABORT, 'observations.source_event_ids elements must be unique');
END;

-- ==== legacy 0104_candidate_claim_event_id_positive_checks.sql ====
CREATE TRIGGER IF NOT EXISTS candidate_claims_event_id_positive_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.event_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'candidate_claims.event_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_event_id_positive_update
BEFORE UPDATE OF event_id ON candidate_claims
WHEN NEW.event_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'candidate_claims.event_id must be positive');
END;

-- ==== legacy 0105_contradiction_claim_id_positive_checks.sql ====
CREATE TRIGGER IF NOT EXISTS contradictions_claim_id_positive_insert
BEFORE INSERT ON contradictions
WHEN NEW.claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'contradictions.claim_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS contradictions_claim_id_positive_update
BEFORE UPDATE OF claim_id ON contradictions
WHEN NEW.claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'contradictions.claim_id must be positive');
END;

-- ==== legacy 0106_contradiction_new_claim_id_positive_checks.sql ====
CREATE TRIGGER IF NOT EXISTS contradictions_new_claim_id_positive_insert
BEFORE INSERT ON contradictions
WHEN NEW.new_claim_id IS NOT NULL AND NEW.new_claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'contradictions.new_claim_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS contradictions_new_claim_id_positive_update
BEFORE UPDATE OF new_claim_id ON contradictions
WHEN NEW.new_claim_id IS NOT NULL AND NEW.new_claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'contradictions.new_claim_id must be positive');
END;

-- ==== legacy 0107_entity_type_parent_id_positive_checks.sql ====
CREATE TRIGGER IF NOT EXISTS entity_types_parent_id_positive_insert
BEFORE INSERT ON entity_types
WHEN NEW.parent_id IS NOT NULL AND NEW.parent_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'entity_types.parent_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS entity_types_parent_id_positive_update
BEFORE UPDATE OF parent_id ON entity_types
WHEN NEW.parent_id IS NOT NULL AND NEW.parent_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'entity_types.parent_id must be positive');
END;

-- ==== legacy 0108_predicate_type_parent_id_positive_checks.sql ====
CREATE TRIGGER IF NOT EXISTS predicate_types_parent_id_positive_insert
BEFORE INSERT ON predicate_types
WHEN NEW.parent_id IS NOT NULL AND NEW.parent_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'predicate_types.parent_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS predicate_types_parent_id_positive_update
BEFORE UPDATE OF parent_id ON predicate_types
WHEN NEW.parent_id IS NOT NULL AND NEW.parent_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'predicate_types.parent_id must be positive');
END;

-- ==== legacy 0109_predicate_alias_predicate_id_positive_checks.sql ====
CREATE TRIGGER IF NOT EXISTS predicate_alias_predicate_id_positive_insert
BEFORE INSERT ON predicate_alias
WHEN NEW.predicate_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'predicate_alias.predicate_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS predicate_alias_predicate_id_positive_update
BEFORE UPDATE OF predicate_id ON predicate_alias
WHEN NEW.predicate_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'predicate_alias.predicate_id must be positive');
END;

-- ==== legacy 0110_entity_type_id_positive_checks.sql ====
CREATE TRIGGER IF NOT EXISTS entities_type_id_positive_insert
BEFORE INSERT ON entities
WHEN NEW.type_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'entities.type_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS entities_type_id_positive_update
BEFORE UPDATE OF type_id ON entities
WHEN NEW.type_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'entities.type_id must be positive');
END;

-- ==== legacy 0111_vector_chunk_claim_id_positive_checks.sql ====
CREATE TRIGGER IF NOT EXISTS vector_chunks_claim_id_positive_insert
BEFORE INSERT ON vector_chunks
WHEN NEW.claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'vector_chunks.claim_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS vector_chunks_claim_id_positive_update
BEFORE UPDATE OF claim_id ON vector_chunks
WHEN NEW.claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'vector_chunks.claim_id must be positive');
END;

-- ==== legacy 0112_candidate_claim_promoted_claim_id_positive_checks.sql ====
CREATE TRIGGER IF NOT EXISTS candidate_claims_promoted_claim_id_positive_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.promoted_claim_id IS NOT NULL AND NEW.promoted_claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_promoted_claim_id_positive_update
BEFORE UPDATE OF promoted_claim_id ON candidate_claims
WHEN NEW.promoted_claim_id IS NOT NULL AND NEW.promoted_claim_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'candidate_claims.promoted_claim_id must be positive');
END;

-- ==== legacy 0113_entity_type_closure_child_id_positive_checks.sql ====
CREATE TRIGGER IF NOT EXISTS entity_type_closure_child_id_positive_insert
BEFORE INSERT ON entity_type_closure
WHEN NEW.child_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'entity_type_closure.child_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS entity_type_closure_child_id_positive_update
BEFORE UPDATE OF child_id ON entity_type_closure
WHEN NEW.child_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'entity_type_closure.child_id must be positive');
END;

-- ==== legacy 0114_entity_type_closure_ancestor_id_positive_checks.sql ====
CREATE TRIGGER IF NOT EXISTS entity_type_closure_ancestor_id_positive_insert
BEFORE INSERT ON entity_type_closure
WHEN NEW.ancestor_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'entity_type_closure.ancestor_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS entity_type_closure_ancestor_id_positive_update
BEFORE UPDATE OF ancestor_id ON entity_type_closure
WHEN NEW.ancestor_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'entity_type_closure.ancestor_id must be positive');
END;

-- ==== legacy 0115_predicate_closure_child_id_positive_checks.sql ====
CREATE TRIGGER IF NOT EXISTS predicate_closure_child_id_positive_insert
BEFORE INSERT ON predicate_closure
WHEN NEW.child_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'predicate_closure.child_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS predicate_closure_child_id_positive_update
BEFORE UPDATE OF child_id ON predicate_closure
WHEN NEW.child_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'predicate_closure.child_id must be positive');
END;

-- ==== legacy 0116_predicate_closure_ancestor_id_positive_checks.sql ====
CREATE TRIGGER IF NOT EXISTS predicate_closure_ancestor_id_positive_insert
BEFORE INSERT ON predicate_closure
WHEN NEW.ancestor_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'predicate_closure.ancestor_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS predicate_closure_ancestor_id_positive_update
BEFORE UPDATE OF ancestor_id ON predicate_closure
WHEN NEW.ancestor_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'predicate_closure.ancestor_id must be positive');
END;

-- ==== legacy 0117_predicate_alias_note_nonblank_checks.sql ====
CREATE TRIGGER IF NOT EXISTS predicate_alias_note_nonblank_insert
BEFORE INSERT ON predicate_alias
WHEN NEW.note IS NOT NULL AND trim(NEW.note) = ''
BEGIN
    SELECT RAISE(ABORT, 'predicate_alias.note must be NULL or nonblank');
END;

CREATE TRIGGER IF NOT EXISTS predicate_alias_note_nonblank_update
BEFORE UPDATE OF note ON predicate_alias
WHEN NEW.note IS NOT NULL AND trim(NEW.note) = ''
BEGIN
    SELECT RAISE(ABORT, 'predicate_alias.note must be NULL or nonblank');
END;

-- ==== legacy 0118_ontology_extension_parent_not_self_checks.sql ====
CREATE TRIGGER IF NOT EXISTS ontology_extension_parent_not_self_insert
BEFORE INSERT ON ontology_extension_log
WHEN NEW.parent = NEW.predicate
BEGIN
    SELECT RAISE(ABORT, 'ontology_extension_log.parent must differ from predicate');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_parent_not_self_update
BEFORE UPDATE OF parent, predicate ON ontology_extension_log
WHEN NEW.parent = NEW.predicate
BEGIN
    SELECT RAISE(ABORT, 'ontology_extension_log.parent must differ from predicate');
END;

-- ==== legacy 0119_hyperedge_predicate_alias_checks.sql ====
-- Hyperedge predicate checks must accept ontology aliases, mirroring the
-- claims trigger (migration 0011) and the Rust-side ontology_check: an
-- aliased predicate (e.g. `has-module`) is accepted for claims but
-- previously aborted hyperedge INSERT/UPDATE at this trigger, after the
-- `add_hyperedge` log line was already appended.
--
-- SQLite has no CREATE OR REPLACE TRIGGER, so drop the 0101 versions and
-- recreate them with the alias clause. Both statements are idempotent so
-- gated re-runs (user_version rewind) stay clean.

DROP TRIGGER IF EXISTS hyperedges_predicate_type_insert;
DROP TRIGGER IF EXISTS hyperedges_predicate_type_update;

CREATE TRIGGER IF NOT EXISTS hyperedges_predicate_type_insert
BEFORE INSERT ON hyperedges
WHEN trim(NEW.predicate) != ''
    AND NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.predicate)
    AND NOT EXISTS (SELECT 1 FROM predicate_alias WHERE alias = NEW.predicate)
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.predicate not in predicate_types');
END;

CREATE TRIGGER IF NOT EXISTS hyperedges_predicate_type_update
BEFORE UPDATE OF predicate ON hyperedges
WHEN trim(NEW.predicate) != ''
    AND NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.predicate)
    AND NOT EXISTS (SELECT 1 FROM predicate_alias WHERE alias = NEW.predicate)
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.predicate not in predicate_types');
END;
