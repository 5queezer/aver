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
