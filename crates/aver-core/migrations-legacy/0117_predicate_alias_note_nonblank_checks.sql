CREATE TRIGGER IF NOT EXISTS predicate_alias_note_nonblank_insert
BEFORE INSERT ON predicate_alias
WHEN NEW.note IS NOT NULL AND trim(NEW.note) = ''
BEGIN
    SELECT RAISE(ABORT, 'predicate_alias.note must be NULL or nonblank');
END;

CREATE TRIGGER IF NOT EXISTS predicate_alias_note_nonblank_update
BEFORE UPDATE OF note ON predicate_alias
WHEN NEW.note IS NOT NULL AND trim(NEW.note) = ''
BEGIN
    SELECT RAISE(ABORT, 'predicate_alias.note must be NULL or nonblank');
END;
