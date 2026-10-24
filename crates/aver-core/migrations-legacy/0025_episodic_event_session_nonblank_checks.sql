-- ADR-0016: episodic events are grouped by a meaningful session id.
-- Store::record_event rejects blank session ids before logging; direct SQL
-- writes should preserve the same recall/audit invariant.
CREATE TRIGGER IF NOT EXISTS episodic_events_session_id_nonblank_insert
BEFORE INSERT ON episodic_events
WHEN trim(NEW.session_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.session_id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS episodic_events_session_id_nonblank_update
BEFORE UPDATE OF session_id ON episodic_events
WHEN trim(NEW.session_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'episodic_events.session_id must not be blank');
END;
