CREATE TRIGGER IF NOT EXISTS hyperedge_participants_hyperedge_id_positive_insert
BEFORE INSERT ON hyperedge_participants
WHEN NEW.hyperedge_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'hyperedge_participants.hyperedge_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS hyperedge_participants_hyperedge_id_positive_update
BEFORE UPDATE OF hyperedge_id ON hyperedge_participants
WHEN NEW.hyperedge_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'hyperedge_participants.hyperedge_id must be positive');
END;
