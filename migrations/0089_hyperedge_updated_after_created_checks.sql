CREATE TRIGGER IF NOT EXISTS hyperedges_updated_after_created_insert
BEFORE INSERT ON hyperedges
WHEN NEW.updated_at < NEW.created_at
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.updated_at must be >= created_at');
END;

CREATE TRIGGER IF NOT EXISTS hyperedges_updated_after_created_update
BEFORE UPDATE OF created_at, updated_at ON hyperedges
WHEN NEW.updated_at < NEW.created_at
BEGIN
    SELECT RAISE(ABORT, 'hyperedges.updated_at must be >= created_at');
END;
