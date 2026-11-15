CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ts_positive_insert
BEFORE INSERT ON observation_prune_markers
WHEN NEW.ts <= 0
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.ts must be positive');
END;

CREATE TRIGGER IF NOT EXISTS observation_prune_markers_ts_positive_update
BEFORE UPDATE OF ts ON observation_prune_markers
WHEN NEW.ts <= 0
BEGIN
    SELECT RAISE(ABORT, 'observation_prune_markers.ts must be positive');
END;
