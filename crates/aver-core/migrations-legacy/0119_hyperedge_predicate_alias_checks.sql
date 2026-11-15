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
