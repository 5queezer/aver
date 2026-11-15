CREATE TRIGGER IF NOT EXISTS ontology_extension_parent_not_self_insert
BEFORE INSERT ON ontology_extension_log
WHEN NEW.parent = NEW.predicate
BEGIN
    SELECT RAISE(ABORT, 'ontology_extension_log.parent must differ from predicate');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_parent_not_self_update
BEFORE UPDATE OF parent, predicate ON ontology_extension_log
WHEN NEW.parent = NEW.predicate
BEGIN
    SELECT RAISE(ABORT, 'ontology_extension_log.parent must differ from predicate');
END;
