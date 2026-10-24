-- Entity type hierarchy edges should not be self-loops. Direct SQL can create
-- a row whose parent_id equals its id, which makes closure/hierarchy semantics
-- ambiguous.
CREATE TRIGGER IF NOT EXISTS entity_types_parent_distinct_insert
BEFORE INSERT ON entity_types
WHEN NEW.parent_id IS NOT NULL AND NEW.parent_id = NEW.id
BEGIN
  SELECT RAISE(ABORT, 'entity_types.parent_id must differ from id');
END;

CREATE TRIGGER IF NOT EXISTS entity_types_parent_distinct_update
BEFORE UPDATE OF id, parent_id ON entity_types
WHEN NEW.parent_id IS NOT NULL AND NEW.parent_id = NEW.id
BEGIN
  SELECT RAISE(ABORT, 'entity_types.parent_id must differ from id');
END;
