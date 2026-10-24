//! Graph traversal: expand, paths, communities, drift snapshot, `GraphStorageAdapter`.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::error::Error;
use crate::store::{Store, scope_filter_sql};
use crate::types::{
    Claim, Community, ConsolidationReport, GraphDriftSnapshot, GraphExpansion, GraphPath,
    GraphPathMode, GraphPathQuery, GraphPathStep, Provenance, RelationshipKind, ScopeWalk,
    StorageMode,
};
use crate::validation::validate_scope;
#[derive(Clone)]
struct EdgeCandidate {
    traverse_from: String,
    traverse_to: String,
    source_id: i64,
    kind_order: u8,
    step: GraphPathStep,
}

/// ADR-0021: hyperedges carry no `scope` column, so the graph layer treats
/// every hyperedge as a `"global"` row. A hyperedge is admitted exactly when
/// the scope filter would admit a claim scoped `"global"`: always for `Any`
/// and `Ancestors` (global is the implicit root of every path), and only for
/// the `"global"` scope itself under `Exact` and `Descendants`.
fn hyperedges_in_scope(scope: &str, walk: ScopeWalk) -> bool {
    match walk {
        ScopeWalk::Any | ScopeWalk::Ancestors => true,
        ScopeWalk::Exact | ScopeWalk::Descendants => scope == "global",
    }
}

pub trait GraphStorageAdapter {
    fn mode(&self) -> StorageMode;
    fn detect_communities(&self) -> Result<Vec<Community>, Error>;
}

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

    /// Find the shortest path between two entities over active claims and
    /// active hyperedges. Directed mode preserves claim direction; explicit
    /// bidirectional mode may traverse claims in reverse. Among shortest paths,
    /// the path with the highest minimum edge confidence is returned.
    ///
    /// ADR-0021: claims are admitted by `query.scope` per `query.walk` using
    /// the same rule as `expand_with_scope`. Hyperedges have no scope column
    /// and are treated as `"global"` rows (see `hyperedges_in_scope`), so they
    /// are reachable from any scope under `Ancestors`/`Any` and only from
    /// `"global"` under `Exact`/`Descendants`. `scope = "global"` with
    /// `ScopeWalk::Any` reproduces the unscoped traversal.
    pub fn graph_path(&self, query: GraphPathQuery) -> Result<Option<GraphPath>, Error> {
        if query.source.trim().is_empty() || query.target.trim().is_empty() {
            return Err(Error::InvalidGraphEntity);
        }
        validate_scope(&query.scope)?;
        if !(0.0..=1.0).contains(&query.min_confidence) {
            return Err(Error::InvalidConfidence {
                value: query.min_confidence,
            });
        }
        if query.source == query.target {
            return Ok(Some(GraphPath::new(
                query.source.clone(),
                query.target.clone(),
                Vec::new(),
                1.0,
                vec![query.source],
            )));
        }
        if query.max_hops == 0 {
            return Ok(None);
        }

        let predicate_filter = if let Some(predicates) = &query.predicates {
            if predicates.is_empty() || predicates.iter().any(|item| item.trim().is_empty()) {
                return Err(Error::InvalidPredicateFilter);
            }
            let borrowed = predicates.iter().map(String::as_str).collect::<Vec<_>>();
            Some(self.expand_predicate_filter(&borrowed)?)
        } else {
            None
        };

        let mut candidates = self.graph_path_candidates(&query, predicate_filter.as_ref())?;
        candidates.sort_by(|left, right| {
            left.source_id
                .cmp(&right.source_id)
                .then_with(|| left.kind_order.cmp(&right.kind_order))
                .then_with(|| left.traverse_from.cmp(&right.traverse_from))
                .then_with(|| left.traverse_to.cmp(&right.traverse_to))
        });

        let mut best: Option<(Vec<GraphPathStep>, Vec<String>, f64)> = None;
        let mut queue = VecDeque::from([(
            query.source.clone(),
            vec![query.source.clone()],
            Vec::<GraphPathStep>::new(),
            1.0_f64,
        )]);
        let mut shortest_depth: Option<usize> = None;

        while let Some((entity, entity_path, steps, path_confidence)) = queue.pop_front() {
            if shortest_depth.is_some_and(|depth| steps.len() >= depth) {
                continue;
            }
            if steps.len() == query.max_hops {
                continue;
            }

            for candidate in candidates
                .iter()
                .filter(|candidate| candidate.traverse_from == entity)
            {
                if entity_path.contains(&candidate.traverse_to) {
                    continue;
                }
                let mut next_entities = entity_path.clone();
                next_entities.push(candidate.traverse_to.clone());
                let mut next_steps = steps.clone();
                next_steps.push(candidate.step.clone());
                let next_confidence = path_confidence * candidate.step.confidence;

                if candidate.traverse_to == query.target {
                    let depth = next_steps.len();
                    shortest_depth = Some(depth);
                    let replace =
                        best.as_ref()
                            .is_none_or(|(best_steps, best_entities, best_confidence)| {
                                next_confidence.total_cmp(best_confidence).is_gt()
                                    || (next_confidence == *best_confidence
                                        && (next_entities.as_slice(), next_steps.len())
                                            < (best_entities.as_slice(), best_steps.len()))
                            });
                    if replace {
                        best = Some((next_steps, next_entities, next_confidence));
                    }
                } else {
                    queue.push_back((
                        candidate.traverse_to.clone(),
                        next_entities,
                        next_steps,
                        next_confidence,
                    ));
                }
            }
        }

        Ok(best.map(|(steps, entity_path, confidence)| {
            GraphPath::new(
                query.source.clone(),
                query.target.clone(),
                steps,
                confidence,
                entity_path,
            )
        }))
    }

    fn graph_path_candidates(
        &self,
        query: &GraphPathQuery,
        predicate_filter: Option<&HashSet<String>>,
    ) -> Result<Vec<EdgeCandidate>, Error> {
        let provenance_allowed = |provenance: Provenance| {
            query
                .allowed_provenance
                .as_ref()
                .is_none_or(|allowed| allowed.contains(&provenance))
        };
        let predicate_allowed =
            |predicate: &str| predicate_filter.is_none_or(|allowed| allowed.contains(predicate));
        let mut candidates = Vec::new();

        let (scope_clause, scope_params) = scope_filter_sql(&query.scope, query.walk);
        let sql = format!(
            "SELECT id
               FROM claims
              WHERE status = 'ACTIVE'
                AND confidence >= ?1
                AND {scope_clause}
              ORDER BY id"
        );
        let mut claim_stmt = self.conn.prepare(&sql)?;
        let mut params: Vec<&dyn rusqlite::ToSql> = vec![&query.min_confidence];
        for s in &scope_params {
            params.push(s);
        }
        let claim_ids = claim_stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                row.get::<_, i64>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(claim_stmt);
        for id in claim_ids {
            let claim = self.get_claim(id)?;
            if !provenance_allowed(claim.provenance) || !predicate_allowed(&claim.predicate) {
                continue;
            }
            let step = GraphPathStep {
                source: claim.subject.clone(),
                target: claim.object.clone(),
                predicate: claim.predicate.clone(),
                confidence: claim.confidence,
                provenance: claim.provenance,
                relationship_kind: RelationshipKind::Claim,
                source_refs: claim.source_refs.clone(),
                claim_id: Some(claim.id),
                scope: claim.scope.clone(),
            };
            candidates.push(EdgeCandidate {
                traverse_from: claim.subject.clone(),
                traverse_to: claim.object.clone(),
                source_id: claim.id,
                kind_order: 0,
                step: step.clone(),
            });
            if query.mode == GraphPathMode::Bidirectional {
                candidates.push(EdgeCandidate {
                    traverse_from: claim.object,
                    traverse_to: claim.subject,
                    source_id: claim.id,
                    kind_order: 0,
                    step,
                });
            }
        }

        if !hyperedges_in_scope(&query.scope, query.walk) {
            return Ok(candidates);
        }
        for hyperedge in self.list_active_hyperedges()? {
            if hyperedge.confidence < query.min_confidence
                || !provenance_allowed(hyperedge.provenance)
                || !predicate_allowed(&hyperedge.predicate)
            {
                continue;
            }
            for (left_index, left) in hyperedge.participants.iter().enumerate() {
                for (right_index, right) in hyperedge.participants.iter().enumerate() {
                    if left_index == right_index || left.entity == right.entity {
                        continue;
                    }
                    candidates.push(EdgeCandidate {
                        traverse_from: left.entity.clone(),
                        traverse_to: right.entity.clone(),
                        source_id: hyperedge.id,
                        kind_order: 1,
                        step: GraphPathStep {
                            source: left.entity.clone(),
                            target: right.entity.clone(),
                            predicate: hyperedge.predicate.clone(),
                            confidence: hyperedge.confidence,
                            provenance: hyperedge.provenance,
                            relationship_kind: RelationshipKind::Hyperedge,
                            source_refs: hyperedge.source_refs.clone(),
                            claim_id: None,
                            scope: "global".to_string(),
                        },
                    });
                }
            }
        }

        Ok(candidates)
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
                    for node in [&claim.subject, &claim.object] {
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
                AND (subject = ?1 OR object = ?1)
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

    /// Weighted connected-component detection over every active claim and
    /// hyperedge; the unscoped case of `detect_communities_with_scope`.
    pub fn detect_communities(&self) -> Result<Vec<Community>, Error> {
        self.detect_communities_with_scope("global", ScopeWalk::Any)
    }

    /// ADR-0021: `detect_communities` restricted to claims admitted by `scope`
    /// per `walk`. Hyperedges have no scope column and are treated as
    /// `"global"` rows (see `hyperedges_in_scope`). Same algorithm, ordering
    /// and scoring as the unscoped case; only the input edge set changes.
    pub fn detect_communities_with_scope(
        &self,
        scope: &str,
        walk: ScopeWalk,
    ) -> Result<Vec<Community>, Error> {
        validate_scope(scope)?;
        #[derive(Debug, Clone)]
        struct WeightedEdge {
            left: String,
            right: String,
            weight: f64,
        }

        const STRONG_EDGE_THRESHOLD: f64 = 0.5;

        fn provenance_weight(provenance: Provenance) -> f64 {
            match provenance {
                Provenance::UserAsserted => 1.0,
                Provenance::Extracted => 0.95,
                Provenance::Inferred => 0.65,
                Provenance::Ambiguous => 0.35,
            }
        }

        fn add_weight(
            weights: &mut BTreeMap<(String, String), f64>,
            left: &str,
            right: &str,
            weight: f64,
        ) {
            if left == right {
                return;
            }
            let key = if left < right {
                (left.to_string(), right.to_string())
            } else {
                (right.to_string(), left.to_string())
            };
            *weights.entry(key).or_insert(0.0) += weight;
        }

        let mut weights: BTreeMap<(String, String), f64> = BTreeMap::new();
        let mut nodes = HashSet::new();

        let (scope_clause, scope_params) = scope_filter_sql(scope, walk);
        let sql = format!(
            "SELECT id
               FROM claims
              WHERE status = 'ACTIVE'
                AND {scope_clause}
              ORDER BY id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let claims = stmt
            .query_map(rusqlite::params_from_iter(scope_params.iter()), |row| {
                row.get::<_, i64>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|id| self.get_claim(id))
            .collect::<Result<Vec<_>, _>>()?;
        for claim in claims {
            nodes.insert(claim.subject.clone());
            nodes.insert(claim.object.clone());
            add_weight(
                &mut weights,
                &claim.subject,
                &claim.object,
                claim.confidence * provenance_weight(claim.provenance),
            );
        }

        let hyperedges = if hyperedges_in_scope(scope, walk) {
            self.list_active_hyperedges()?
        } else {
            Vec::new()
        };
        for hyperedge in hyperedges {
            for participant in &hyperedge.participants {
                nodes.insert(participant.entity.clone());
            }
            for left_idx in 0..hyperedge.participants.len() {
                for right_idx in (left_idx + 1)..hyperedge.participants.len() {
                    add_weight(
                        &mut weights,
                        &hyperedge.participants[left_idx].entity,
                        &hyperedge.participants[right_idx].entity,
                        hyperedge.confidence * provenance_weight(hyperedge.provenance),
                    );
                }
            }
        }

        let edges: Vec<WeightedEdge> = weights
            .into_iter()
            .map(|((left, right), weight)| WeightedEdge {
                left,
                right,
                weight: weight.min(1.0),
            })
            .collect();

        let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
        for node in &nodes {
            adjacency.entry(node.clone()).or_default();
        }
        for edge in &edges {
            if edge.weight >= STRONG_EDGE_THRESHOLD {
                adjacency
                    .entry(edge.left.clone())
                    .or_default()
                    .push(edge.right.clone());
                adjacency
                    .entry(edge.right.clone())
                    .or_default()
                    .push(edge.left.clone());
            }
        }
        for neighbours in adjacency.values_mut() {
            neighbours.sort();
        }

        let mut seen = HashSet::new();
        let mut communities = Vec::new();
        let mut sorted_nodes: Vec<String> = nodes.into_iter().collect();
        sorted_nodes.sort();

        for node in sorted_nodes {
            if !seen.insert(node.clone()) {
                continue;
            }
            let mut members = Vec::new();
            let mut queue = VecDeque::from([node]);
            while let Some(current) = queue.pop_front() {
                members.push(current.clone());
                for neighbour in adjacency.get(&current).into_iter().flatten() {
                    if seen.insert(neighbour.clone()) {
                        queue.push_back(neighbour.clone());
                    }
                }
            }
            members.sort();
            let member_set: HashSet<&str> = members.iter().map(String::as_str).collect();
            let internal_edges: Vec<&WeightedEdge> = edges
                .iter()
                .filter(|edge| {
                    member_set.contains(edge.left.as_str())
                        && member_set.contains(edge.right.as_str())
                })
                .collect();
            let score = if internal_edges.is_empty() {
                0.0
            } else {
                internal_edges.iter().map(|edge| edge.weight).sum::<f64>()
                    / internal_edges.len() as f64
            };
            let mut bridge_nodes: Vec<String> = members
                .iter()
                .filter(|member| {
                    edges.iter().any(|edge| {
                        (edge.left == **member && !member_set.contains(edge.right.as_str()))
                            || (edge.right == **member && !member_set.contains(edge.left.as_str()))
                    })
                })
                .cloned()
                .collect();
            bridge_nodes.sort();
            let id = format!("community:{}", members.join("-"));
            communities.push(Community {
                id,
                members,
                score,
                bridge_nodes,
            });
        }

        communities.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(communities)
    }
}

impl GraphStorageAdapter for Store {
    fn mode(&self) -> StorageMode {
        StorageMode::Local
    }

    fn detect_communities(&self) -> Result<Vec<Community>, Error> {
        Store::detect_communities(self)
    }
}
