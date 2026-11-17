//! ADR-0033: opt-in bounded graph-first recall.
//!
//! Anchors on entities the query names (at most [`MAX_ANCHORS`]), expands
//! from them breadth-first under [`MAX_EDGES`] and `hops`, then selects
//! facts inside that bounded set by a deterministic route. Source events are
//! read only for the selected claims. `None` means no anchor or no edge was
//! found, and the caller falls back to hybrid recall.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use crate::claims::SqlParams;
use crate::error::Error;
use crate::objects::ObjectKind;
use crate::recall::tokenize_for_recall;
use crate::retrieval::HybridWeights;
use crate::store::Store;
use crate::types::{Claim, EpisodicEvent, GraphExpansion, RecallFilters};
use crate::validation::validate_recall_query;

pub const MAX_ANCHORS: usize = 15;
pub const MAX_EDGES: usize = 64;
/// Fact cap for the list and count routes, which ignore `top_k`.
pub const LIST_LIMIT: usize = 25;

const COUNT_CUES: &[&str] = &["count", "number"];
const TEMPORAL_CUES: &[&str] = &[
    "when", "latest", "recent", "recently", "last", "first", "before", "after", "since", "until",
    "newest", "oldest", "history", "changed",
];
const INTERSECTION_CUES: &[&str] = &["both", "and", "between", "common", "shared"];
const LIST_CUES: &[&str] = &["list", "which", "all", "every", "enumerate"];
const STOPWORDS: &[&str] = &[
    "a",
    "an",
    "the",
    "of",
    "to",
    "in",
    "on",
    "for",
    "by",
    "with",
    "at",
    "from",
    "or",
    "is",
    "are",
    "was",
    "were",
    "be",
    "do",
    "doe",
    "does",
    "did",
    "what",
    "who",
    "whom",
    "how",
    "many",
    "much",
    "most",
    "we",
    "i",
    "it",
    "its",
    "that",
    "this",
    "there",
    "thing",
    "everything",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryRoute {
    Single,
    Temporal,
    Intersection,
    List,
    Count,
}

impl QueryRoute {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Temporal => "temporal",
            Self::Intersection => "intersection",
            Self::List => "list",
            Self::Count => "count",
        }
    }
}

fn words(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// Whole-word cues in fixed precedence: count, temporal, intersection (only
/// with two or more anchors), list, else single.
pub fn route_query(query: &str, anchor_count: usize) -> QueryRoute {
    let words = words(query);
    let has = |cues: &[&str]| words.iter().any(|w| cues.contains(&w.as_str()));
    let phrase = |a: &str, b: &str| words.windows(2).any(|w| w[0] == a && w[1] == b);
    if has(COUNT_CUES) || phrase("how", "many") {
        QueryRoute::Count
    } else if has(TEMPORAL_CUES) {
        QueryRoute::Temporal
    } else if anchor_count >= 2 && has(INTERSECTION_CUES) {
        QueryRoute::Intersection
    } else if has(LIST_CUES) || phrase("what", "are") {
        QueryRoute::List
    } else {
        QueryRoute::Single
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphFirstOptions {
    pub hops: usize,
    pub top_k: usize,
    /// Only the recency term is read; it re-ranks inside the bounded set.
    pub weights: HybridWeights,
}

impl Default for GraphFirstOptions {
    fn default() -> Self {
        Self {
            hops: 2,
            top_k: 5,
            weights: HybridWeights::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GraphFirstFact {
    pub claim: Claim,
    /// The anchor whose expansion reached this claim first.
    pub anchor: String,
    /// 1 for a claim touching the anchor, 2 for one hop further, ...
    pub hop: usize,
    /// `valid_from` when the claim has one, else its write time.
    pub ts: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphFirstBudget {
    pub anchors: usize,
    pub max_anchors: usize,
    pub hops: usize,
    pub edges_visited: usize,
    pub max_edges: usize,
    /// Claims left after the route narrowed the expansion, before the limit.
    pub candidates: usize,
    pub selected: usize,
    /// More anchors or edges existed than the budget allowed.
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct GraphFirstRecall {
    pub route: QueryRoute,
    pub anchors: Vec<String>,
    pub facts: Vec<GraphFirstFact>,
    /// Source events of the selected facts, in fact order, same scope only.
    pub events: Vec<EpisodicEvent>,
    /// Count route only: the number of matching claims in the bounded set.
    pub count: Option<usize>,
    pub budget: GraphFirstBudget,
    pub subgraph: GraphExpansion,
}

struct Edge {
    claim: Claim,
    anchor: usize,
    hop: usize,
}

/// Token `t` from the query matches claim token `c` exactly or as a prefix
/// of at least three characters ("own" matches "owned").
fn token_match(t: &str, c: &str) -> bool {
    t == c || (t.len() >= 3 && c.starts_with(t)) || (c.len() >= 3 && t.starts_with(c))
}

impl Store {
    pub fn recall_graph_first(
        &self,
        query: &str,
        filters: &RecallFilters,
        options: &GraphFirstOptions,
        now: i64,
    ) -> Result<Option<GraphFirstRecall>, Error> {
        validate_recall_query(query)?;
        self.validate_filters(filters)?;
        if options.hops == 0 {
            return Err(Error::InvalidGraphHops);
        }
        let query_tokens: Vec<String> = tokenize_for_recall(query)
            .into_iter()
            .filter(|t| !STOPWORDS.contains(&t.as_str()))
            .collect();
        let (anchors, more_anchors) = self.graph_first_anchors(&query_tokens, filters)?;
        if anchors.is_empty() {
            return Ok(None);
        }
        let (nodes, edges, reach, more_edges) =
            self.bounded_expand(&anchors, options.hops, filters)?;
        if edges.is_empty() {
            return Ok(None);
        }

        // Tokens that say something beyond the anchors' names.
        let anchor_tokens: HashSet<String> = anchors
            .iter()
            .flat_map(|a| tokenize_for_recall(a))
            .collect();
        let content: Vec<&String> = query_tokens
            .iter()
            .filter(|t| !anchor_tokens.contains(*t))
            .collect();
        let matches = |claim: &Claim| {
            let tokens = tokenize_for_recall(&format!("{} {}", claim.predicate, claim.object));
            content
                .iter()
                .filter(|t| tokens.iter().any(|c| token_match(t, c)))
                .count()
        };
        let recency = HybridWeights {
            alpha: 0.0,
            ..options.weights
        };
        let score = |edge: &Edge| {
            let relevance = if content.is_empty() {
                0.0
            } else {
                matches(&edge.claim) as f64 / content.len() as f64
            };
            recency.blend_with_age(
                0.0,
                relevance + 0.5 / edge.hop as f64,
                edge.claim.verified_age_secs(now),
            )
        };
        let ts = |claim: &Claim| claim.valid_from.unwrap_or(claim.write_ts);

        let route = route_query(query, anchors.len());
        let mut selected: Vec<&Edge> = edges.iter().collect();
        // List, count and temporal narrow to claims matching the content
        // words when any do; an empty match keeps the whole set.
        if matches!(
            route,
            QueryRoute::List | QueryRoute::Count | QueryRoute::Temporal
        ) {
            let matching: Vec<&Edge> = selected
                .iter()
                .copied()
                .filter(|e| matches(&e.claim) > 0)
                .collect();
            if !matching.is_empty() {
                selected = matching;
            }
        }
        if route == QueryRoute::Intersection {
            let anchor_set: HashSet<&String> = anchors.iter().collect();
            let joins = |edge: &&Edge| {
                let ends = endpoints(&edge.claim);
                (ends.len() == 2 && ends.iter().all(|n| anchor_set.contains(n)))
                    || ends.iter().any(|n| {
                        !anchor_set.contains(n) && reach.get(*n).is_some_and(|r| r.len() >= 2)
                    })
            };
            let joined: Vec<&Edge> = selected.iter().copied().filter(joins).collect();
            if !joined.is_empty() {
                selected = joined;
            }
        }
        let mut scored: Vec<(f64, &Edge)> = selected.into_iter().map(|e| (score(e), e)).collect();
        scored.sort_by(|(a_score, a), (b_score, b)| {
            let by_time = if route == QueryRoute::Temporal {
                ts(&b.claim).cmp(&ts(&a.claim))
            } else {
                std::cmp::Ordering::Equal
            };
            by_time
                .then_with(|| b_score.total_cmp(a_score))
                .then_with(|| a.claim.id.cmp(&b.claim.id))
        });
        let candidates = scored.len();
        let count = (route == QueryRoute::Count).then_some(candidates);
        let limit = match route {
            QueryRoute::List | QueryRoute::Count => LIST_LIMIT,
            _ => options.top_k,
        };
        scored.truncate(limit);

        let facts: Vec<GraphFirstFact> = scored
            .into_iter()
            .map(|(_, edge)| GraphFirstFact {
                ts: ts(&edge.claim),
                claim: edge.claim.clone(),
                anchor: anchors[edge.anchor].clone(),
                hop: edge.hop,
            })
            .collect();
        let mut events = Vec::new();
        let mut seen_events = HashSet::new();
        for fact in &facts {
            if let Some(id) = fact.claim.event_id
                && seen_events.insert(id)
            {
                let event = self.get_event(id)?;
                if event.scope == fact.claim.scope {
                    events.push(event);
                }
            }
        }
        Ok(Some(GraphFirstRecall {
            route,
            budget: GraphFirstBudget {
                anchors: anchors.len(),
                max_anchors: MAX_ANCHORS,
                hops: options.hops,
                edges_visited: edges.len(),
                max_edges: MAX_EDGES,
                candidates,
                selected: facts.len(),
                truncated: more_anchors || more_edges,
            },
            anchors,
            facts,
            events,
            count,
            subgraph: GraphExpansion {
                nodes,
                edges: edges.into_iter().map(|e| e.claim).collect(),
            },
        }))
    }

    /// Entities (subjects and entity objects passing `filters`) at least half
    /// of whose tokens appear in the query, best coverage first, capped at
    /// [`MAX_ANCHORS`]. The flag says whether more qualified.
    fn graph_first_anchors(
        &self,
        query_tokens: &[String],
        filters: &RecallFilters,
    ) -> Result<(Vec<String>, bool), Error> {
        if query_tokens.is_empty() {
            return Ok((Vec::new(), false));
        }
        let mut entities = BTreeSet::new();
        for sql in [
            "SELECT DISTINCT subject FROM claims WHERE ",
            "SELECT DISTINCT object FROM claims WHERE object_kind = 'entity' AND ",
        ] {
            let Some((filter_sql, params)) = self.claim_filter_sql(filters)? else {
                return Ok((Vec::new(), false));
            };
            entities.extend(self.query_strings(&format!("{sql}{filter_sql}"), params)?);
        }
        let query: HashSet<&str> = query_tokens.iter().map(String::as_str).collect();
        let mut ranked: Vec<(f64, BTreeSet<String>, String)> = entities
            .into_iter()
            .filter_map(|entity| {
                let tokens: HashSet<String> = tokenize_for_recall(&entity).into_iter().collect();
                let matched: BTreeSet<String> = tokens
                    .iter()
                    .filter(|t| query.contains(t.as_str()))
                    .cloned()
                    .collect();
                let coverage = matched.len() as f64 / tokens.len().max(1) as f64;
                (!matched.is_empty() && coverage >= 0.5).then_some((coverage, matched, entity))
            })
            .collect();
        // An entity whose matched words are a strict subset of another's is
        // not named by the query: in "PaymentGateway", "payment" belongs to
        // PaymentGateway, not team_payments.
        let sets: Vec<BTreeSet<String>> = ranked.iter().map(|(_, m, _)| m.clone()).collect();
        ranked.retain(|(_, m, _)| !sets.iter().any(|other| m.is_subset(other) && m != other));
        ranked.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then(b.1.len().cmp(&a.1.len()))
                .then(a.2.cmp(&b.2))
        });
        let more = ranked.len() > MAX_ANCHORS;
        ranked.truncate(MAX_ANCHORS);
        Ok((ranked.into_iter().map(|(_, _, e)| e).collect(), more))
    }

    fn query_strings(&self, sql: &str, params: SqlParams) -> Result<Vec<String>, Error> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
            |row| row.get::<_, String>(0),
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Breadth-first from every anchor in rank order, sharing one edge
    /// budget. Returns the visited nodes, the claims with their first
    /// anchor and hop, which anchors reached each node, and whether the
    /// budget cut the expansion short.
    #[allow(clippy::type_complexity)]
    fn bounded_expand(
        &self,
        anchors: &[String],
        hops: usize,
        filters: &RecallFilters,
    ) -> Result<
        (
            Vec<String>,
            Vec<Edge>,
            HashMap<String, BTreeSet<usize>>,
            bool,
        ),
        Error,
    > {
        let mut nodes = Vec::new();
        let mut reach: HashMap<String, BTreeSet<usize>> = HashMap::new();
        let mut queue = VecDeque::new();
        for (i, anchor) in anchors.iter().enumerate() {
            nodes.push(anchor.clone());
            reach.entry(anchor.clone()).or_default().insert(i);
            queue.push_back((anchor.clone(), i, 0usize));
        }
        let mut adjacency: HashMap<String, Vec<i64>> = HashMap::new();
        let mut edges: Vec<Edge> = Vec::new();
        let mut edge_index: HashMap<i64, usize> = HashMap::new();
        let mut truncated = false;

        while let Some((node, anchor, depth)) = queue.pop_front() {
            if depth == hops {
                continue;
            }
            if !adjacency.contains_key(&node) {
                // Already-seen edges count toward the LIMIT, so fetch a full
                // budget plus one: any unseen edge past the budget is seen.
                let claims = self.adjacent_claims(&node, filters, MAX_EDGES + 1)?;
                let mut ids = Vec::new();
                for claim in claims {
                    if let Some(&i) = edge_index.get(&claim.id) {
                        ids.push(edges[i].claim.id);
                    } else if edges.len() < MAX_EDGES {
                        edge_index.insert(claim.id, edges.len());
                        ids.push(claim.id);
                        edges.push(Edge {
                            claim,
                            anchor,
                            hop: depth + 1,
                        });
                    } else {
                        truncated = true;
                    }
                }
                adjacency.insert(node.clone(), ids);
            }
            for id in adjacency[&node].clone() {
                let ends: Vec<String> = endpoints(&edges[edge_index[&id]].claim)
                    .into_iter()
                    .cloned()
                    .collect();
                for end in ends {
                    if reach.entry(end.clone()).or_default().insert(anchor) {
                        if !nodes.contains(&end) {
                            nodes.push(end.clone());
                        }
                        queue.push_back((end, anchor, depth + 1));
                    }
                }
            }
        }
        Ok((nodes, edges, reach, truncated))
    }

    /// Up to `limit` claims touching `entity` that pass `filters`, by id.
    fn adjacent_claims(
        &self,
        entity: &str,
        filters: &RecallFilters,
        limit: usize,
    ) -> Result<Vec<Claim>, Error> {
        let Some((filter_sql, filter_params)) = self.claim_filter_sql(filters)? else {
            return Ok(Vec::new());
        };
        let mut params: SqlParams =
            vec![Box::new(entity.to_string()), Box::new(entity.to_string())];
        params.extend(filter_params);
        params.push(Box::new(limit as i64));
        self.claims_where(
            &format!(
                "id IN (SELECT id FROM claims
                         WHERE (subject = ? OR (object_kind = 'entity' AND object = ?))
                           AND {filter_sql}
                         ORDER BY id LIMIT ?)"
            ),
            params,
        )
    }
}

fn endpoints(claim: &Claim) -> Vec<&String> {
    let mut ends = vec![&claim.subject];
    if claim.object_kind == ObjectKind::Entity && claim.object != claim.subject {
        ends.push(&claim.object);
    }
    ends
}
