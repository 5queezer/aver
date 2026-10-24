-- ADR-0006/ADR-0005: claims.source_refs is parsed as Vec<String>.
-- The JSON array shape is enforced by 0012; this additive migration tightens
-- direct-SQL writes so array elements are text references, not numbers/objects.
CREATE TRIGGER IF NOT EXISTS claims_source_refs_text_insert
BEFORE INSERT ON claims
WHEN json_valid(NEW.source_refs) = 1
 AND json_type(NEW.source_refs) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_refs)
        WHERE type != 'text'
     )
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs elements must be text');
END;

CREATE TRIGGER IF NOT EXISTS claims_source_refs_text_update
BEFORE UPDATE OF source_refs ON claims
WHEN json_valid(NEW.source_refs) = 1
 AND json_type(NEW.source_refs) = 'array'
 AND EXISTS (
       SELECT 1
         FROM json_each(NEW.source_refs)
        WHERE type != 'text'
     )
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs elements must be text');
END;
