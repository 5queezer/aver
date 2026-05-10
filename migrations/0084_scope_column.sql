-- ADR-0021: scope as a first-class memory dimension.
--
-- Add `scope TEXT NOT NULL DEFAULT 'global'` to the four memory-bearing
-- tables (claims, episodic_events, observations, candidate_claims), with
-- per-table indexes and BEFORE INSERT/UPDATE triggers that enforce the
-- charset `[A-Za-z0-9_/-]` and reject blanks.
--
-- Migration is purely additive. Existing rows default to 'global', which
-- is the correct scope for the two ACTIVE claims observed at the time of
-- ADR-0021 (vasudev-core / is_about and user / prefers_code_review_model).

ALTER TABLE claims            ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';
ALTER TABLE episodic_events   ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';
ALTER TABLE observations      ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';
ALTER TABLE candidate_claims  ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';

CREATE INDEX IF NOT EXISTS claims_scope            ON claims(scope);
CREATE INDEX IF NOT EXISTS episodic_events_scope   ON episodic_events(scope);
CREATE INDEX IF NOT EXISTS observations_scope      ON observations(scope);
CREATE INDEX IF NOT EXISTS candidate_claims_scope  ON candidate_claims(scope);

-- Charset trigger pattern mirrors `claims_agent_id_charset_*` from
-- migration 0061: GLOB negation across the allowed character class.
-- Allowed: A-Z a-z 0-9 underscore hyphen forward-slash.

CREATE TRIGGER IF NOT EXISTS claims_scope_nonblank_insert
BEFORE INSERT ON claims
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS claims_scope_nonblank_update
BEFORE UPDATE OF scope ON claims
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS claims_scope_charset_insert
BEFORE INSERT ON claims
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'claims.scope contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS claims_scope_charset_update
BEFORE UPDATE OF scope ON claims
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'claims.scope contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_scope_nonblank_insert
BEFORE INSERT ON episodic_events
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_scope_nonblank_update
BEFORE UPDATE OF scope ON episodic_events
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_scope_charset_insert
BEFORE INSERT ON episodic_events
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.scope contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_scope_charset_update
BEFORE UPDATE OF scope ON episodic_events
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.scope contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS observations_scope_nonblank_insert
BEFORE INSERT ON observations
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observations_scope_nonblank_update
BEFORE UPDATE OF scope ON observations
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'observations.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS observations_scope_charset_insert
BEFORE INSERT ON observations
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'observations.scope contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS observations_scope_charset_update
BEFORE UPDATE OF scope ON observations
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'observations.scope contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_scope_nonblank_insert
BEFORE INSERT ON candidate_claims
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_scope_nonblank_update
BEFORE UPDATE OF scope ON candidate_claims
WHEN trim(NEW.scope) = ''
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.scope must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_scope_charset_insert
BEFORE INSERT ON candidate_claims
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.scope contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS candidate_claims_scope_charset_update
BEFORE UPDATE OF scope ON candidate_claims
WHEN trim(NEW.scope) != ''
 AND NEW.scope GLOB '*[^A-Za-z0-9_/-]*'
BEGIN
  SELECT RAISE(ABORT, 'candidate_claims.scope contains invalid characters');
END;
