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
