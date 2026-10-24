-- Keep ontology extension agent ids aligned with Store agent-id validation:
-- nonblank ASCII alphanumeric plus underscore or hyphen only.
CREATE TRIGGER IF NOT EXISTS ontology_extension_agent_id_charset_insert
BEFORE INSERT ON ontology_extension_log
WHEN trim(NEW.agent_id) != '' AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.agent_id contains invalid characters');
END;

CREATE TRIGGER IF NOT EXISTS ontology_extension_agent_id_charset_update
BEFORE UPDATE OF agent_id ON ontology_extension_log
WHEN trim(NEW.agent_id) != '' AND NEW.agent_id GLOB '*[^A-Za-z0-9_-]*'
BEGIN
  SELECT RAISE(ABORT, 'ontology_extension_log.agent_id contains invalid characters');
END;
