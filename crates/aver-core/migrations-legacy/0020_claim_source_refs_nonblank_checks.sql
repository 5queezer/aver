-- ADR-0006/ADR-0005: claim provenance references are meaningful strings.
-- Store::add_claim rejects blank sources; these triggers preserve the same
-- invariant for direct SQL JSON-array updates.
CREATE TRIGGER IF NOT EXISTS claims_source_refs_nonblank_insert
BEFORE INSERT ON claims
WHEN json_valid(NEW.source_refs) = 1
 AND json_type(NEW.source_refs) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_refs)
        WHERE type = 'text' AND trim(value) = ''
     )
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs elements must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS claims_source_refs_nonblank_update
BEFORE UPDATE OF source_refs ON claims
WHEN json_valid(NEW.source_refs) = 1
 AND json_type(NEW.source_refs) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_refs)
        WHERE type = 'text' AND trim(value) = ''
     )
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs elements must not be blank');
END;
