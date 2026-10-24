-- ADR-0016: episodic event kind is the event classifier used by extraction
-- and summaries. Store::record_event rejects blank kinds; direct SQL writes
-- should preserve the same typed event-stream invariant.
CREATE TRIGGER IF NOT EXISTS episodic_events_kind_nonblank_insert
BEFORE INSERT ON episodic_events
WHEN trim(NEW.kind) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.kind must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_kind_nonblank_update
BEFORE UPDATE OF kind ON episodic_events
WHEN trim(NEW.kind) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.kind must not be blank');
END;
