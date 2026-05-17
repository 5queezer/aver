CREATE TRIGGER IF NOT EXISTS hyperedges_source_refs_unique_insert
BEFORE INSERT ON hyperedges
WHEN json_valid(NEW.source_refs)
    AND json_type(NEW.source_refs) = 'array'
    AND (
        SELECT COUNT(*) != COUNT(DISTINCT json_each.value)
        FROM json_each(NEW.source_refs)
        WHERE json_each.type = 'text'
    )
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.source_refs elements must be unique');
END;

CREATE TRIGGER IF NOT EXISTS hyperedges_source_refs_unique_update
BEFORE UPDATE OF source_refs ON hyperedges
WHEN json_valid(NEW.source_refs)
    AND json_type(NEW.source_refs) = 'array'
    AND (
        SELECT COUNT(*) != COUNT(DISTINCT json_each.value)
        FROM json_each(NEW.source_refs)
        WHERE json_each.type = 'text'
    )
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.source_refs elements must be unique');
END;
