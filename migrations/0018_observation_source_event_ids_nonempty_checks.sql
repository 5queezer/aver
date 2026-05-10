-- ADR-0016: every durable observation must cite at least one source event.
-- Store::record_observation already enforces this; these triggers keep direct
-- SQL writes from creating unsupported observations that recall cannot audit.
CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_nonempty_insert
BEFORE INSERT ON observations
WHEN json_valid(NEW.source_event_ids) = 1
 AND json_type(NEW.source_event_ids) = 'array'
 AND json_array_length(NEW.source_event_ids) = 0
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids must not be empty');
END;

CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_nonempty_update
BEFORE UPDATE OF source_event_ids ON observations
WHEN json_valid(NEW.source_event_ids) = 1
 AND json_type(NEW.source_event_ids) = 'array'
 AND json_array_length(NEW.source_event_ids) = 0
BEGIN
  SELECT RAISE(ABORT, 'observations.source_event_ids must not be empty');
END;
