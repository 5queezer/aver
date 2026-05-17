CREATE TRIGGER IF NOT EXISTS hyperedges_source_refs_nonempty_insert
BEFORE INSERT ON hyperedges
WHEN json_valid(NEW.source_refs) AND json_type(NEW.source_refs) = 'array' AND json_array_length(NEW.source_refs) = 0
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.source_refs must not be empty');
END;

CREATE TRIGGER IF NOT EXISTS hyperedges_source_refs_nonempty_update
BEFORE UPDATE OF source_refs ON hyperedges
WHEN json_valid(NEW.source_refs) AND json_type(NEW.source_refs) = 'array' AND json_array_length(NEW.source_refs) = 0
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.source_refs must not be empty');
END;
