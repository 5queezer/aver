-- ADR-0016: observation source_event_ids is stored as JSON text and
-- replay/recall code expects a JSON array of event ids. Keep this additive
-- for existing databases by using triggers rather than rebuilding the table.
CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_json_array_insert
BEFORE INSERT ON observations
WHEN CASE
       WHEN json_valid(NEW.source_event_ids) = 0 THEN 1
       WHEN json_type(NEW.source_event_ids) != 'array' THEN 1
       ELSE 0
     END
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids must be a JSON array');
END;

CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_json_array_update
BEFORE UPDATE OF source_event_ids ON observations
WHEN CASE
       WHEN json_valid(NEW.source_event_ids) = 0 THEN 1
       WHEN json_type(NEW.source_event_ids) != 'array' THEN 1
       ELSE 0
     END
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids must be a JSON array');
END;
