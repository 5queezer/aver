CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_json_array_insert
BEFORE INSERT ON observation_prune_markers
WHEN NOT (json_valid(NEW.pruned_observation_ids) AND json_type(NEW.pruned_observation_ids) = 'array')
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids must be a JSON array');
END;

CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_json_array_update
BEFORE UPDATE OF pruned_observation_ids ON observation_prune_markers
WHEN NOT (json_valid(NEW.pruned_observation_ids) AND json_type(NEW.pruned_observation_ids) = 'array')
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids must be a JSON array');
END;
