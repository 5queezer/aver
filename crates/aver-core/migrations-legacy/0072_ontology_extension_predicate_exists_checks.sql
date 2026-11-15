-- Ontology extension log predicate values should refer to predicate vocabulary
-- terms that actually exist. This keeps audit rows tied to the row added by
-- the extension path even for direct SQL writes.
CREATE TRIGGER IF NOT EXISTS ontology_extension_predicate_exists_insert
BEFORE INSERT ON ontology_extension_log
WHEN trim(NEW.predicate) != ''
 AND NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.predicate)
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.predicate must exist in predicate_types');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_predicate_exists_update
BEFORE UPDATE OF predicate ON ontology_extension_log
WHEN trim(NEW.predicate) != ''
 AND NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.predicate)
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.predicate must exist in predicate_types');
END;
