CREATE TRIGGER IF NOT EXISTS candidate_claims_event_id_positive_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.event_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'candidate_claims.event_id must be positive');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_event_id_positive_update
BEFORE UPDATE OF event_id ON candidate_claims
WHEN NEW.event_id <= 0
BEGIN
    SELECT RAISE(ABORT, 'candidate_claims.event_id must be positive');
END;
