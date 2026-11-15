-- Predicate aliases are alternate vocabulary identifiers accepted by claim
-- ontology enforcement. Reject blank aliases so direct SQL cannot create an
-- alias that would make blank predicates appear meaningful.
CREATE TRIGGER IF NOT EXISTS predicate_alias_nonblank_insert
BEFORE INSERT ON predicate_alias
WHEN trim(NEW.alias) = ''
BEGIN
  SELECT RAISE(ABORT, 'predicate_alias.alias must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS predicate_alias_nonblank_update
BEFORE UPDATE OF alias ON predicate_alias
WHEN trim(NEW.alias) = ''
BEGIN
  SELECT RAISE(ABORT, 'predicate_alias.alias must not be blank');
END;
