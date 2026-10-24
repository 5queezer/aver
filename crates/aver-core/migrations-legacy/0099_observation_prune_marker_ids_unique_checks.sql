CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_unique_insert
BEFORE INSERT ON observation_prune_markers
WHEN json_valid(NEW.pruned_observation_ids)
    AND json_type(NEW.pruned_observation_ids) = 'array'
    AND (
        SELECT COUNT(*) != COUNT(DISTINCT json_each.value)
        FROM json_each(NEW.pruned_observation_ids)
        WHERE json_each.type = 'text'
    )
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids elements must be unique');
END;

CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_unique_update
BEFORE UPDATE OF pruned_observation_ids ON observation_prune_markers
WHEN json_valid(NEW.pruned_observation_ids)
    AND json_type(NEW.pruned_observation_ids) = 'array'
    AND (
        SELECT COUNT(*) != COUNT(DISTINCT json_each.value)
        FROM json_each(NEW.pruned_observation_ids)
        WHERE json_each.type = 'text'
    )
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids elements must be unique');
END;
