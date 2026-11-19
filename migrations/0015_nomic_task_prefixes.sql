-- ADR-0017 amendment: nomic document vectors predate search_document prefixes.
-- Preserve their text for an explicit, convention-tagged backfill.
DELETE FROM vector_index
 WHERE chunk_id IN (
       SELECT id FROM vector_chunks
        WHERE embedding_model GLOB 'nomic-embed-text*'
 );

UPDATE vector_chunks
   SET embedding_json = NULL
 WHERE embedding_model GLOB 'nomic-embed-text*';
