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
