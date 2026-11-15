-- Preserve legacy projection meaning until an explicit logged migration.
ALTER TABLE claims ADD COLUMN object_kind TEXT NOT NULL DEFAULT 'entity'
    CHECK (object_kind IN ('entity', 'literal'));
