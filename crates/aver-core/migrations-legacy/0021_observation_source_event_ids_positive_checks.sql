-- ADR-0016: observation source_event_ids cite episodic_events.id values,
-- which are positive row ids. Store::record_observation verifies existence;
-- this trigger blocks obviously invalid non-positive ids for direct SQL rows.
CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_positive_insert
BEFORE INSERT ON observations
WHEN json_valid(NEW.source_event_ids) = 1
 AND json_type(NEW.source_event_ids) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_event_ids)
        WHERE type = 'integer' AND value <= 0
     )
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids elements must be positive');
END;

CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_positive_update
BEFORE UPDATE OF source_event_ids ON observations
WHEN json_valid(NEW.source_event_ids) = 1
 AND json_type(NEW.source_event_ids) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_event_ids)
        WHERE type = 'integer' AND value <= 0
     )
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids elements must be positive');
END;
