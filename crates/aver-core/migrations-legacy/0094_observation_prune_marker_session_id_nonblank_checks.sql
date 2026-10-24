CREATE TRIGGER IF NOT EXISTS observation_prune_markers_session_id_nonblank_insert
BEFORE INSERT ON observation_prune_markers
WHEN trim(NEW.session_id) = ''
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.session_id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observation_prune_markers_session_id_nonblank_update
BEFORE UPDATE OF session_id ON observation_prune_markers
WHEN trim(NEW.session_id) = ''
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.session_id must not be blank');
END;
