-- Ontology extension log entries must name the parent vocabulary term used to
-- seat a user-asserted predicate. Reject blank parents so audit records explain
-- the extension path.
CREATE TRIGGER IF NOT EXISTS ontology_extension_parent_nonblank_insert
BEFORE INSERT ON ontology_extension_log
WHEN trim(NEW.parent) = ''
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.parent must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_parent_nonblank_update
BEFORE UPDATE OF parent ON ontology_extension_log
WHEN trim(NEW.parent) = ''
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.parent must not be blank');
END;
