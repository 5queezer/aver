CREATE UNIQUE INDEX IF NOT EXISTS hyperedge_participants_hyperedge_role_entity
ON hyperedge_participants(hyperedge_id, role, entity);

CREATE TRIGGER IF NOT EXISTS hyperedge_participants_duplicate_role_entity_insert
BEFORE INSERT ON hyperedge_participants
WHEN EXISTS (
    SELECT 1
    FROM hyperedge_participants existing
    WHERE existing.hyperedge_id = NEW.hyperedge_id
      AND existing.role = NEW.role
      AND existing.entity = NEW.entity
)
BEGIN
    SELECT RAISE(ABORT, 'hyperedge_participants role/entity must be unique per hyperedge');
END;

CREATE TRIGGER IF NOT EXISTS hyperedge_participants_duplicate_role_entity_update
BEFORE UPDATE OF hyperedge_id, role, entity ON hyperedge_participants
WHEN EXISTS (
    SELECT 1
    FROM hyperedge_participants existing
    WHERE existing.id != NEW.id
      AND existing.hyperedge_id = NEW.hyperedge_id
      AND existing.role = NEW.role
      AND existing.entity = NEW.entity
)
BEGIN
    SELECT RAISE(ABORT, 'hyperedge_participants role/entity must be unique per hyperedge');
END;
