-- ADR-0016: observations are scoped to a concrete session for compaction and
-- replay. Store::record_observation rejects blank session ids; direct SQL rows
-- should preserve the same session provenance invariant.
CREATE TRIGGER IF NOT EXISTS observations_session_id_nonblank_insert
BEFORE INSERT ON observations
WHEN trim(NEW.session_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.session_id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observations_session_id_nonblank_update
BEFORE UPDATE OF session_id ON observations
WHEN trim(NEW.session_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.session_id must not be blank');
END;
