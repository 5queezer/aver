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
