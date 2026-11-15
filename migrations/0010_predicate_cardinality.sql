-- ADR-0010/0026: predicate cardinality.
--
-- Consolidation used to treat every predicate as single-valued: an ACTIVE
-- claim with a newer claim on the same (subject, predicate) and a different
-- object was superseded, so `File:x defines A` lost to `File:x defines B`.
-- `single` predicates hold one object per subject (the newer value wins at
-- consolidation); `multi` predicates accumulate. Everything defaults to
-- `multi`, including predicates added by ADR-0018 ontology extension; the
-- seed (`seed.rs`) marks the single-valued ones on every open.
ALTER TABLE predicate_types
  ADD COLUMN cardinality TEXT NOT NULL DEFAULT 'multi'
  CHECK (cardinality IN ('single', 'multi'));
