CREATE TRIGGER IF NOT EXISTS hyperedges_source_refs_elements_nonblank_insert
BEFORE INSERT ON hyperedges
WHEN json_valid(NEW.source_refs)
    AND json_type(NEW.source_refs) = 'array'
    AND EXISTS (
        SELECT 1
        FROM json_each(NEW.source_refs)
        WHERE json_each.type != 'text' OR trim(json_each.value) = ''
    )
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.source_refs elements must be nonblank strings');
END;

CREATE TRIGGER IF NOT EXISTS hyperedges_source_refs_elements_nonblank_update
BEFORE UPDATE OF source_refs ON hyperedges
WHEN json_valid(NEW.source_refs)
    AND json_type(NEW.source_refs) = 'array'
    AND EXISTS (
        SELECT 1
        FROM json_each(NEW.source_refs)
        WHERE json_each.type != 'text' OR trim(json_each.value) = ''
    )
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.source_refs elements must be nonblank strings');
END;
