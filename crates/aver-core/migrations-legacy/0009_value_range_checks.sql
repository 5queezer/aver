CREATE TRIGGER IF NOT EXISTS claims_confidence_range_insert
BEFORE INSERT ON claims
WHEN NEW.confidence < 0.0 OR NEW.confidence > 1.0
BEGIN
  SELECT RAISE(ABORT, 'claims.confidence must be in [0, 1]');
END;

CREATE TRIGGER IF NOT EXISTS claims_confidence_range_update
BEFORE UPDATE OF confidence ON claims
WHEN NEW.confidence < 0.0 OR NEW.confidence > 1.0
BEGIN
  SELECT RAISE(ABORT, 'claims.confidence must be in [0, 1]');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_confidence_range_insert
BEFORE INSERT ON candidate_claims
WHEN NEW.confidence < 0.0 OR NEW.confidence > 1.0
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.confidence must be in [0, 1]');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_confidence_range_update
BEFORE UPDATE OF confidence ON candidate_claims
WHEN NEW.confidence < 0.0 OR NEW.confidence > 1.0
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.confidence must be in [0, 1]');
END;

CREATE TRIGGER IF NOT EXISTS claims_status_enum_insert
BEFORE INSERT ON claims
WHEN NEW.status NOT IN ('ACTIVE', 'SUPERSEDED', 'INVALIDATED')
BEGIN
  SELECT RAISE(ABORT, 'claims.status must be ACTIVE, SUPERSEDED, or INVALIDATED');
END;

CREATE TRIGGER IF NOT EXISTS claims_status_enum_update
BEFORE UPDATE OF status ON claims
WHEN NEW.status NOT IN ('ACTIVE', 'SUPERSEDED', 'INVALIDATED')
BEGIN
  SELECT RAISE(ABORT, 'claims.status must be ACTIVE, SUPERSEDED, or INVALIDATED');
END;
