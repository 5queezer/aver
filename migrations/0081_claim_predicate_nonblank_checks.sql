-- Durable claim predicates are ontology vocabulary identifiers. Reject blank
-- direct SQL writes with a precise error before ontology membership checks.
CREATE TRIGGER IF NOT EXISTS claims_predicate_nonblank_insert
BEFORE INSERT ON claims
WHEN trim(NEW.predicate) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.predicate must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS claims_predicate_nonblank_update
BEFORE UPDATE OF predicate ON claims
WHEN trim(NEW.predicate) = ''
BEGIN
  SELECT RAISE(ABORT, 'claims.predicate must not be blank');
END;
