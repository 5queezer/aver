-- ADR-0016: observations.source_event_ids is read as Vec<i64>; direct SQL
-- rows with string/object/null elements would make recall parsing fail later.
-- Keep this as an additive trigger-based invariant for existing databases.
CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_integer_insert
BEFORE INSERT ON observations
WHEN json_valid(NEW.source_event_ids) = 1
 AND json_type(NEW.source_event_ids) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_event_ids)
        WHERE type != 'integer'
     )
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids elements must be integers');
END;

CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_integer_update
BEFORE UPDATE OF source_event_ids ON observations
WHEN json_valid(NEW.source_event_ids) = 1
 AND json_type(NEW.source_event_ids) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_event_ids)
        WHERE type != 'integer'
     )
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids elements must be integers');
END;
