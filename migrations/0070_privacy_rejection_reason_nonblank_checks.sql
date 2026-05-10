-- Privacy rejection telemetry stores aggregate reason labels only. Reject blank
-- direct SQL rows so counters remain actionable without containing secret data.
CREATE TRIGGER IF NOT EXISTS privacy_rejections_reason_nonblank_insert
BEFORE INSERT ON privacy_rejections
WHEN trim(NEW.reason) = ''
BEGIN
  SELECT RAISE(ABORT, 'privacy_rejections.reason must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS privacy_rejections_reason_nonblank_update
BEFORE UPDATE OF reason ON privacy_rejections
WHEN trim(NEW.reason) = ''
BEGIN
  SELECT RAISE(ABORT, 'privacy_rejections.reason must not be blank');
END;
