CREATE TRIGGER IF NOT EXISTS observation_prune_markers_id_nonblank_insert
BEFORE INSERT ON observation_prune_markers
WHEN trim(NEW.id) = ''
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observation_prune_markers_id_nonblank_update
BEFORE UPDATE OF id ON observation_prune_markers
WHEN trim(NEW.id) = ''
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.id must not be blank');
END;
