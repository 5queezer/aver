CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_nonempty_insert
BEFORE INSERT ON observation_prune_markers
WHEN json_valid(NEW.pruned_observation_ids)
    AND json_type(NEW.pruned_observation_ids) = 'array'
    AND json_array_length(NEW.pruned_observation_ids) = 0
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids must not be empty');
END;

CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_nonempty_update
BEFORE UPDATE OF pruned_observation_ids ON observation_prune_markers
WHEN json_valid(NEW.pruned_observation_ids)
    AND json_type(NEW.pruned_observation_ids) = 'array'
    AND json_array_length(NEW.pruned_observation_ids) = 0
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids must not be empty');
END;
