-- ADR-0016: observations must explain how they were derived from source
-- events. Store::record_observation rejects blank derivations before logging;
-- direct SQL writes should preserve that auditability invariant.
CREATE TRIGGER IF NOT EXISTS observations_derivation_nonblank_insert
BEFORE INSERT ON observations
WHEN trim(NEW.derivation) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.derivation must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observations_derivation_nonblank_update
BEFORE UPDATE OF derivation ON observations
WHEN trim(NEW.derivation) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.derivation must not be blank');
END;
