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
