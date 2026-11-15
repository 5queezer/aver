CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_elements_nonblank_insert
BEFORE INSERT ON observation_prune_markers
WHEN json_valid(NEW.pruned_observation_ids)
    AND json_type(NEW.pruned_observation_ids) = 'array'
    AND EXISTS (
        SELECT 1
        FROM json_each(NEW.pruned_observation_ids)
        WHERE json_each.type != 'text' OR trim(json_each.value) = ''
    )
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids elements must be nonblank strings');
END;

CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ids_elements_nonblank_update
BEFORE UPDATE OF pruned_observation_ids ON observation_prune_markers
WHEN json_valid(NEW.pruned_observation_ids)
    AND json_type(NEW.pruned_observation_ids) = 'array'
    AND EXISTS (
        SELECT 1
        FROM json_each(NEW.pruned_observation_ids)
        WHERE json_each.type != 'text' OR trim(json_each.value) = ''
    )
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.pruned_observation_ids elements must be nonblank strings');
END;
