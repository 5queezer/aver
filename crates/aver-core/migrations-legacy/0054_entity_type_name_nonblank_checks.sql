-- Ontology entity type names are vocabulary identifiers. Reject blank direct
-- SQL inserts/updates so closure rows and classification logic never reference
-- an unnamed type.
CREATE TRIGGER IF NOT EXISTS entity_types_name_nonblank_insert
BEFORE INSERT ON entity_types
WHEN trim(NEW.name) = ''
BEGIN
  SELECT RAISE(ABORT, 'entity_types.name must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS entity_types_name_nonblank_update
BEFORE UPDATE OF name ON entity_types
WHEN trim(NEW.name) = ''
BEGIN
  SELECT RAISE(ABORT, 'entity_types.name must not be blank');
END;
