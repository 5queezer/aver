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
