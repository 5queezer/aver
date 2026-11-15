-- Observation ids are deterministic identifiers for source-backed projections.
-- Blank ids cannot be retrieved or audited meaningfully; keep direct SQL rows
-- aligned with Store-generated observation ids.
CREATE TRIGGER IF NOT EXISTS observations_id_nonblank_insert
BEFORE INSERT ON observations
WHEN trim(NEW.id) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observations_id_nonblank_update
BEFORE UPDATE OF id ON observations
WHEN trim(NEW.id) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.id must not be blank');
END;
