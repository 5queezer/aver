CREATE TRIGGER IF NOT EXISTS hyperedges_predicate_nonblank_insert
BEFORE INSERT ON hyperedges
WHEN trim(NEW.predicate) = ''
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.predicate must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS hyperedges_predicate_nonblank_update
BEFORE UPDATE OF predicate ON hyperedges
WHEN trim(NEW.predicate) = ''
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.predicate must not be blank');
END;
