//! Graph traversal: expand and drift snapshots.

use std::collections::{BTreeMap, HashSet, VecDeque};

use crate::error::Error;
use crate::store::{Store, scope_filter_sql};
use crate::types::{
    Claim, ConsolidationReport, GraphDriftSnapshot, GraphExpansion, Provenance, ScopeWalk,
};
use crate::validation::validate_scope;
impl Store {
    pub fn graph_drift_snapshot(
        &self,
        consolidation: ConsolidationReport,
    ) -> Result<GraphDriftSnapshot, Error> {
        let mut claim_count_by_provenance = BTreeMap::new();
        let mut mean_confidence_by_provenance = BTreeMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT provenance, COUNT(*), AVG(confidence)
               FROM claims GROUP BY provenance ORDER BY provenance",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })?;
        let mut total_claims = 0_u64;
        let mut ambiguous_claims = 0_u64;
        for row in rows {
            let (provenance, count, mean_confidence) = row?;
            if provenance == Provenance::Ambiguous.as_str() {
                ambiguous_claims = count;
            }
            total_claims += count;
            claim_count_by_provenance.insert(provenance.clone(), count);
            mean_confidence_by_provenance.insert(provenance, mean_confidence);
        }

        let contradicts_edge_count =
            self.conn
                .query_row("SELECT COUNT(*) FROM contradictions", [], |row| {
                    row.get::<_, u64>(0)
                })?;
        let mut entity_count_by_type_id = BTreeMap::new();
        let mut entity_stmt = self.conn.prepare(
            "SELECT entity_types.name, COUNT(entities.name)
               FROM entities JOIN entity_types ON entities.type_id = entity_types.id
              GROUP BY entity_types.name ORDER BY entity_types.name",
        )?;
        let entity_rows = entity_stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?;
        for row in entity_rows {
            let (type_id, count) = row?;
            entity_count_by_type_id.insert(type_id, count);
        }

        let mut privacy_rejection_counts = BTreeMap::new();
        let mut privacy_stmt = self
            .conn
            .prepare("SELECT reason, count FROM privacy_rejections ORDER BY reason")?;
        let privacy_rows = privacy_stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?;
        for row in privacy_rows {
            let (reason, count) = row?;
            privacy_rejection_counts.insert(reason, count);
        }

        Ok(GraphDriftSnapshot {
            claim_count_by_provenance,
            mean_confidence_by_provenance,
            contradicts_edge_count,
            ambiguous_ratio: if total_claims == 0 {
                0.0
            } else {
                ambiguous_claims as f64 / total_claims as f64
            },
            entity_count_by_type_id,
            consolidation_merged: consolidation.merged,
            consolidation_superseded: consolidation.superseded,
            privacy_rejection_counts,
        })
    }

    /// Text-only keyword recall over active claims. This is the v0.1
    /// precursor to HybridRAG: cheap SQLite substring matching across the
    /// claim triple fields, ordered deterministically by id.
    pub fn expand(
        &self,
        entity: &str,
        hops: usize,
        predicates: Option<&[&str]>,
    ) -> Result<GraphExpansion, Error> {
        self.expand_inner(entity, hops, predicates, "global", ScopeWalk::Any)
    }

    /// ADR-0021: variant of `expand` that filters the graph by scope per walk.
    pub fn expand_with_scope(
        &self,
        entity: &str,
        hops: usize,
        predicates: Option<&[&str]>,
        scope: &str,
        walk: ScopeWalk,
    ) -> Result<GraphExpansion, Error> {
        validate_scope(scope)?;
        self.expand_inner(entity, hops, predicates, scope, walk)
    }

    fn expand_inner(
        &self,
        entity: &str,
        hops: usize,
        predicates: Option<&[&str]>,
        scope: &str,
        walk: ScopeWalk,
    ) -> Result<GraphExpansion, Error> {
        if entity.trim().is_empty() {
            return Err(Error::InvalidGraphEntity);
        }
        if hops == 0 {
            return Err(Error::InvalidGraphHops);
        }

        let predicate_filter = if let Some(items) = predicates {
            if items.is_empty() || items.iter().any(|item| item.trim().is_empty()) {
                return Err(Error::InvalidPredicateFilter);
            }
            Some(self.expand_predicate_filter(items)?)
        } else {
            None
        };
        let mut nodes = vec![entity.to_string()];
        let mut seen_nodes = HashSet::from([entity.to_string()]);
        let mut seen_edges = HashSet::new();
        let mut queue = VecDeque::from([(entity.to_string(), 0usize)]);
        let mut edges = Vec::new();

        while let Some((current, depth)) = queue.pop_front() {
            if depth == hops {
                continue;
            }
            for claim in self.active_claim_edges_for_entity(&current, scope, walk)? {
                if predicate_filter
                    .as_ref()
                    .is_some_and(|allowed| !allowed.contains(claim.predicate.as_str()))
                {
                    continue;
                }
                if seen_edges.insert(claim.id) {
                    for node in std::iter::once(&claim.subject).chain(
                        (claim.object_kind == crate::ObjectKind::Entity).then_some(&claim.object),
                    ) {
                        if seen_nodes.insert(node.clone()) {
                            nodes.push(node.clone());
                            queue.push_back((node.clone(), depth + 1));
                        }
                    }
                    edges.push(claim);
                }
            }
        }

        Ok(GraphExpansion { nodes, edges })
    }

    fn active_claim_edges_for_entity(
        &self,
        entity: &str,
        scope: &str,
        walk: ScopeWalk,
    ) -> Result<Vec<Claim>, Error> {
        let (scope_clause, scope_params) = scope_filter_sql(scope, walk);
        let sql = format!(
            "SELECT id
               FROM claims
              WHERE status = 'ACTIVE'
                AND (subject = ?1 OR (object_kind = 'entity' AND object = ?1))
                AND {scope_clause}
              ORDER BY id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut params: Vec<&dyn rusqlite::ToSql> = vec![&entity];
        for s in &scope_params {
            params.push(s);
        }
        let ids = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                row.get::<_, i64>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter().map(|id| self.get_claim(id)).collect()
    }
}
