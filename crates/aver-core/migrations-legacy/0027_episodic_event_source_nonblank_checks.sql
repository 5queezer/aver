-- ADR-0016: episodic events need a source label for auditability.
-- Store::record_event rejects blank sources before logging; direct SQL writes
-- should preserve the same provenance invariant.
CREATE TRIGGER IF NOT EXISTS episodic_events_source_nonblank_insert
BEFORE INSERT ON episodic_events
WHEN trim(NEW.source) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.source must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_source_nonblank_update
BEFORE UPDATE OF source ON episodic_events
WHEN trim(NEW.source) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.source must not be blank');
END;
