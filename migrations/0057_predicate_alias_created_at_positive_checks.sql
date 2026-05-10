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
