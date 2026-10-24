-- Ontology extension log parent values should refer to seeded or extended
-- predicate vocabulary terms. This keeps audit rows tied to a real ontology
-- seating point even for direct SQL writes.
CREATE TRIGGER IF NOT EXISTS ontology_extension_parent_exists_insert
BEFORE INSERT ON ontology_extension_log
WHEN trim(NEW.parent) != ''
 AND NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.parent)
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.parent must exist in predicate_types');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_parent_exists_update
BEFORE UPDATE OF parent ON ontology_extension_log
WHEN trim(NEW.parent) != ''
 AND NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.parent)
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.parent must exist in predicate_types');
END;
