-- Ontology predicate type names are vocabulary identifiers. Reject blank direct
-- SQL inserts/updates so claim predicate validation never accepts unnamed
-- predicate types.
CREATE TRIGGER IF NOT EXISTS predicate_types_name_nonblank_insert
BEFORE INSERT ON predicate_types
WHEN trim(NEW.name) = ''
BEGIN
  SELECT RAISE(ABORT, 'predicate_types.name must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS predicate_types_name_nonblank_update
BEFORE UPDATE OF name ON predicate_types
WHEN trim(NEW.name) = ''
BEGIN
  SELECT RAISE(ABORT, 'predicate_types.name must not be blank');
END;
