-- Ontology extension timestamps come from runtime/migration Unix seconds.
-- Reject non-positive sentinel timestamps from direct SQL so extension audit
-- chronology remains meaningful.
CREATE TRIGGER IF NOT EXISTS ontology_extension_created_at_positive_insert
BEFORE INSERT ON ontology_extension_log
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.created_at must be positive');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_created_at_positive_update
BEFORE UPDATE OF created_at ON ontology_extension_log
WHEN NEW.created_at <= 0
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.created_at must be positive');
END;
