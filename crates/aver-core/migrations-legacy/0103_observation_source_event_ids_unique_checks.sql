CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_unique_insert
BEFORE INSERT ON observations
WHEN json_valid(NEW.source_event_ids)
    AND json_type(NEW.source_event_ids) = 'array'
    AND (
        SELECT COUNT(*) != COUNT(DISTINCT json_each.value)
        FROM json_each(NEW.source_event_ids)
        WHERE json_each.type = 'integer'
    )
BEGIN
    SELECT RAISE(ABORT, 'observations.source_event_ids elements must be unique');
END;

CREATE TRIGGER IF NOT EXISTS observations_source_event_ids_unique_update
BEFORE UPDATE OF source_event_ids ON observations
WHEN json_valid(NEW.source_event_ids)
    AND json_type(NEW.source_event_ids) = 'array'
    AND (
        SELECT COUNT(*) != COUNT(DISTINCT json_each.value)
        FROM json_each(NEW.source_event_ids)
        WHERE json_each.type = 'integer'
    )
BEGIN
    SELECT RAISE(ABORT, 'observations.source_event_ids elements must be unique');
END;
