-- Direct links validate at write time and after full replay (events load later).
ALTER TABLE claims ADD COLUMN event_id INTEGER;
CREATE INDEX claims_event_id ON claims(event_id) WHERE event_id IS NOT NULL;
