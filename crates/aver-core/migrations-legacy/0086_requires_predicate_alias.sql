-- Add a semantic predicate alias for model-authored dependency claims.
-- `requires` means the same relationship as canonical `depends_on`, but
-- keeping it as an alias preserves source/log fidelity for callers that emit
-- natural-language relation names.
--
-- `depends_on` is normally seeded by seed_ontology(), which runs after all
-- migrations. We INSERT OR IGNORE it here so the alias foreign-key resolves
-- on fresh databases where the seed has not yet run.
INSERT OR IGNORE INTO predicate_types (name, parent_id, created_via, created_at)
VALUES ('depends_on', NULL, 'migration', strftime('%s','now'));

INSERT OR IGNORE INTO predicate_alias (alias, predicate_id, created_at, note)
SELECT 'requires', id, strftime('%s','now'), 'semantic alias for depends_on'
  FROM predicate_types
 WHERE name = 'depends_on';
