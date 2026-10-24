-- Ontology extension log entries must name the agent responsible for the
-- user-asserted vocabulary addition. Reject blank agent ids in direct SQL so
-- the extension audit trail remains attributable.
CREATE TRIGGER IF NOT EXISTS ontology_extension_agent_id_nonblank_insert
BEFORE INSERT ON ontology_extension_log
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.agent_id must not be blank');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_agent_id_nonblank_update
BEFORE UPDATE OF agent_id ON ontology_extension_log
WHEN trim(NEW.agent_id) = ''
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.agent_id must not be blank');
END;
