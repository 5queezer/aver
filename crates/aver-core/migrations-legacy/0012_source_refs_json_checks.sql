-- ADR-0006/ADR-0005: claim source_refs is stored as JSON text but must
-- remain a replayable JSON array. SQLite cannot add a CHECK constraint to
-- an existing table without rebuilding it, so this additive migration uses
-- triggers for defense-in-depth on direct SQL writes.
CREATE TRIGGER IF NOT EXISTS claims_source_refs_json_array_insert
BEFORE INSERT ON claims
WHEN CASE
       WHEN json_valid(NEW.source_refs) = 0 THEN 1
       WHEN json_type(NEW.source_refs) != 'array' THEN 1
       ELSE 0
     END
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs must be a JSON array');
END;

CREATE TRIGGER IF NOT EXISTS claims_source_refs_json_array_update
BEFORE UPDATE OF source_refs ON claims
WHEN CASE
       WHEN json_valid(NEW.source_refs) = 0 THEN 1
       WHEN json_type(NEW.source_refs) != 'array' THEN 1
       ELSE 0
     END
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs must be a JSON array');
END;
