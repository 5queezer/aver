-- Privacy rejection telemetry counts aggregate observed rejections. Reject
-- non-positive direct SQL rows so telemetry snapshots stay meaningful.
CREATE TRIGGER IF NOT EXISTS privacy_rejections_count_positive_insert
BEFORE INSERT ON privacy_rejections
WHEN NEW.count <= 0
BEGIN
  SELECT RAISE(ABORT, 'privacy_rejections.count must be positive');
END;

CREATE TRIGGER IF NOT EXISTS privacy_rejections_count_positive_update
BEFORE UPDATE OF count ON privacy_rejections
WHEN NEW.count <= 0
BEGIN
  SELECT RAISE(ABORT, 'privacy_rejections.count must be positive');
END;
