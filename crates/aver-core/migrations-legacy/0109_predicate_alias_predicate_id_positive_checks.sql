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
