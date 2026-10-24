CREATE TRIGGER IF NOT EXISTS hyperedges_predicate_type_insert
BEFORE INSERT ON hyperedges
WHEN trim(NEW.predicate) != ''
    AND NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.predicate)
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.predicate not in predicate_types');
END;

CREATE TRIGGER IF NOT EXISTS hyperedges_predicate_type_update
BEFORE UPDATE OF predicate ON hyperedges
WHEN trim(NEW.predicate) != ''
    AND NOT EXISTS (SELECT 1 FROM predicate_types WHERE name = NEW.predicate)
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.predicate not in predicate_types');
END;
