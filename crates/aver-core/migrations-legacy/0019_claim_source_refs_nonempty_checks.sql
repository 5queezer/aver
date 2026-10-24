-- ADR-0006/ADR-0005: durable claims must retain at least one provenance
-- reference. Store::add_claim writes one source ref; these triggers prevent
-- direct SQL from stripping claim auditability with an empty JSON array.
CREATE TRIGGER IF NOT EXISTS claims_source_refs_nonempty_insert
BEFORE INSERT ON claims
WHEN json_valid(NEW.source_refs) = 1
 AND json_type(NEW.source_refs) = 'array'
 AND json_array_length(NEW.source_refs) = 0
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs must not be empty');
END;

CREATE TRIGGER IF NOT EXISTS claims_source_refs_nonempty_update
BEFORE UPDATE OF source_refs ON claims
WHEN json_valid(NEW.source_refs) = 1
 AND json_type(NEW.source_refs) = 'array'
 AND json_array_length(NEW.source_refs) = 0
BEGIN
  SELECT RAISE(ABORT, 'claims.source_refs must not be empty');
END;
