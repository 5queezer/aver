-- Contradictions are audit records explaining why a claim is challenged.
-- Store::contradict validates nonblank reasons; direct SQL writes should not
-- create unexplained contradiction records.
CREATE TRIGGER IF NOT EXISTS contradictions_reason_nonblank_insert
BEFORE INSERT ON contradictions
WHEN trim(NEW.reason) = ''
BEGIN
  SELECT RAISE(ABORT, 'contradictions.reason must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS contradictions_reason_nonblank_update
BEFORE UPDATE OF reason ON contradictions
WHEN trim(NEW.reason) = ''
BEGIN
  SELECT RAISE(ABORT, 'contradictions.reason must not be blank');
END;
