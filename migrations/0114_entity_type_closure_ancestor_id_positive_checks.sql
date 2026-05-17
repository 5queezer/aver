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
