-- Ontology extension log entries audit user-asserted vocabulary additions.
-- Reject blank predicates so the audit log always names the extended
-- vocabulary term.
CREATE TRIGGER IF NOT EXISTS ontology_extension_predicate_nonblank_insert
BEFORE INSERT ON ontology_extension_log
WHEN trim(NEW.predicate) = ''
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.predicate must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_predicate_nonblank_update
BEFORE UPDATE OF predicate ON ontology_extension_log
WHEN trim(NEW.predicate) = ''
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.predicate must not be blank');
END;
