-- ADR-0016: observations are source-backed semantic projections. Blank
-- content is not a meaningful projection and Store::record_observation rejects
-- it before logging; direct SQL writes should preserve that invariant.
CREATE TRIGGER IF NOT EXISTS observations_content_nonblank_insert
BEFORE INSERT ON observations
WHEN trim(NEW.content) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.content must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observations_content_nonblank_update
BEFORE UPDATE OF content ON observations
WHEN trim(NEW.content) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.content must not be blank');
END;
