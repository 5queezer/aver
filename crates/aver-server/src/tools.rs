use std::path::Path;

use aver_core::{
    AgentKind, CandidateClaim, Claim, ContradictionRecord, EpisodicEvent, NewClaim, Store,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RememberClaimParams {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RecallParams {
    pub query: String,
    #[serde(default)]
    pub alpha: Option<f64>,
    #[serde(default)]
    pub hops: Option<usize>,
    #[serde(default)]
    pub top_k: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ExpandParams {
    pub entity: String,
    #[serde(default)]
    pub hops: Option<usize>,
    #[serde(default)]
    pub predicates: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct AddTripleParams {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    #[serde(default)]
    pub confidence: Option<f64>,
    pub source: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ContradictNewClaimParams {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub source: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ContradictParams {
    pub triple_id: i64,
    pub reason: String,
    #[serde(default)]
    pub new_claim: Option<ContradictNewClaimParams>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ConsolidateParams {
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RecordEventParams {
    pub session_id: String,
    pub kind: String,
    pub payload: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ShouldExtractMemoriesParams {
    pub session_id: String,
    pub event_threshold: usize,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ProposeCandidateClaimParams {
    pub event_id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ListCandidateClaimsParams {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct PromoteCandidateClaimParams {
    pub candidate_id: i64,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct RejectCandidateClaimParams {
    pub candidate_id: i64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ClaimView {
    pub id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f64,
    pub status: String,
    pub source_refs: Vec<String>,
    pub agent_id: String,
    pub agent_kind: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EventView {
    pub id: i64,
    pub session_id: String,
    pub kind: String,
    pub payload: String,
    pub source: String,
    pub agent_id: String,
    pub agent_kind: String,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CandidateClaimView {
    pub id: i64,
    pub event_id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub provenance: String,
    pub confidence: f64,
    pub status: String,
    pub promoted_claim_id: Option<i64>,
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RecallView {
    pub triples: Vec<ClaimView>,
    pub chunks: Vec<String>,
    pub subgraph: GraphView,
    pub confidence_floor: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GraphView {
    pub nodes: Vec<String>,
    pub edges: Vec<ClaimView>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AddTripleView {
    pub triple_id: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ContradictionView {
    pub contradiction_id: i64,
    pub claim_id: i64,
    pub reason: String,
    pub new_claim_id: Option<i64>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConsolidateView {
    pub merged: usize,
    pub superseded: usize,
    pub decayed: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ShouldExtractMemoriesView {
    pub should_extract: bool,
}

impl From<Claim> for ClaimView {
    fn from(claim: Claim) -> Self {
        Self {
            id: claim.id,
            subject: claim.subject,
            predicate: claim.predicate,
            object: claim.object,
            confidence: claim.confidence,
            status: claim.status.as_str().to_string(),
            source_refs: claim.source_refs,
            agent_id: claim.agent_id,
            agent_kind: claim.agent_kind.as_str().to_string(),
        }
    }
}

impl From<EpisodicEvent> for EventView {
    fn from(event: EpisodicEvent) -> Self {
        Self {
            id: event.id,
            session_id: event.session_id,
            kind: event.kind,
            payload: event.payload,
            source: event.source,
            agent_id: event.agent_id,
            agent_kind: event.agent_kind.as_str().to_string(),
            ts: event.ts,
        }
    }
}

impl From<CandidateClaim> for CandidateClaimView {
    fn from(candidate: CandidateClaim) -> Self {
        Self {
            id: candidate.id,
            event_id: candidate.event_id,
            subject: candidate.subject,
            predicate: candidate.predicate,
            object: candidate.object,
            provenance: candidate.provenance.as_str().to_string(),
            confidence: candidate.confidence,
            status: candidate.status,
            promoted_claim_id: candidate.promoted_claim_id,
            rejection_reason: candidate.rejection_reason,
        }
    }
}

impl From<ContradictionRecord> for ContradictionView {
    fn from(record: ContradictionRecord) -> Self {
        Self {
            contradiction_id: record.id,
            claim_id: record.claim_id,
            reason: record.reason,
            new_claim_id: record.new_claim_id,
            status: record.status.to_ascii_lowercase(),
        }
    }
}

pub struct AverTools {
    store: Store,
}

impl AverTools {
    pub fn open(memory_dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        Ok(Self {
            store: Store::open(memory_dir)?,
        })
    }

    pub fn remember_claim(&self, params: RememberClaimParams) -> anyhow::Result<ClaimView> {
        if params.subject.trim().is_empty() {
            anyhow::bail!("invalid subject: must not be empty");
        }
        if params.predicate.trim().is_empty() {
            anyhow::bail!("invalid predicate: must not be empty");
        }
        if params.object.trim().is_empty() {
            anyhow::bail!("invalid object: must not be empty");
        }
        let source = params.source.as_deref().unwrap_or("mcp");
        let agent_id = params.agent_id.as_deref().unwrap_or("mcp");
        let agent_kind = params
            .agent_kind
            .as_deref()
            .unwrap_or("EXTERNAL_TOOL")
            .parse::<AgentKind>()?;
        let id = self.store.add_claim_from_agent(
            agent_id,
            agent_kind,
            &params.subject,
            &params.predicate,
            &params.object,
            source,
        )?;
        Ok(self.store.get_claim(id)?.into())
    }

    pub fn recall(&self, params: RecallParams) -> anyhow::Result<RecallView> {
        let top_k = validate_top_k(params.top_k.unwrap_or(5))?;
        let mut claims = self.store.recall_text(&params.query)?;
        claims.truncate(top_k);
        let _alpha = if let Some(alpha) = params.alpha {
            aver_core::retrieval::HybridWeights::try_new(alpha)
                .map_err(|err| anyhow::anyhow!("invalid alpha: {err}"))?
                .alpha
        } else {
            aver_core::retrieval::HybridWeights::for_query(&params.query).alpha
        };
        let hops = validate_hops(params.hops.unwrap_or(2))?;
        let mut subgraph = self.store.expand(&params.query, hops, None)?;
        if subgraph.edges.is_empty()
            && let Some(first_claim) = claims.first()
        {
            subgraph = self.store.expand(&first_claim.subject, hops, None)?;
        }
        let confidence_floor = claims
            .iter()
            .chain(subgraph.edges.iter())
            .map(|claim| claim.confidence)
            .min_by(f64::total_cmp)
            .unwrap_or(0.0);
        Ok(RecallView {
            triples: claims.into_iter().map(ClaimView::from).collect(),
            chunks: Vec::new(),
            subgraph: GraphView {
                nodes: subgraph.nodes,
                edges: subgraph.edges.into_iter().map(ClaimView::from).collect(),
            },
            confidence_floor,
        })
    }

    pub fn expand(&self, params: ExpandParams) -> anyhow::Result<GraphView> {
        let hops = validate_hops(params.hops.unwrap_or(2))?;
        let predicate_refs = params
            .predicates
            .as_ref()
            .map(|items| items.iter().map(String::as_str).collect::<Vec<_>>());
        let graph = self
            .store
            .expand(&params.entity, hops, predicate_refs.as_deref())?;
        Ok(GraphView {
            nodes: graph.nodes,
            edges: graph.edges.into_iter().map(ClaimView::from).collect(),
        })
    }

    pub fn add_triple(&self, params: AddTripleParams) -> anyhow::Result<AddTripleView> {
        if params.subject.trim().is_empty() {
            anyhow::bail!("invalid subject: must not be empty");
        }
        if params.predicate.trim().is_empty() {
            anyhow::bail!("invalid predicate: must not be empty");
        }
        if params.object.trim().is_empty() {
            anyhow::bail!("invalid object: must not be empty");
        }
        if let Some(confidence) = params.confidence
            && !(0.0..=1.0).contains(&confidence)
        {
            anyhow::bail!("invalid confidence: must be between 0 and 1");
        }
        let id = self.store.add_claim_with_confidence(
            &params.subject,
            &params.predicate,
            &params.object,
            &params.source,
            params.confidence.unwrap_or(0.95),
        )?;
        Ok(AddTripleView {
            triple_id: id,
            status: "appended".to_string(),
        })
    }

    pub fn contradict(&self, params: ContradictParams) -> anyhow::Result<ContradictionView> {
        let new_claim = params.new_claim.as_ref().map(|claim| NewClaim {
            subject: claim.subject.as_str(),
            predicate: claim.predicate.as_str(),
            object: claim.object.as_str(),
            source: claim.source.as_str(),
        });
        Ok(self
            .store
            .contradict(params.triple_id, &params.reason, new_claim)?
            .into())
    }

    pub fn consolidate(&self, params: ConsolidateParams) -> anyhow::Result<ConsolidateView> {
        if let Some(scope) = params.scope.as_deref()
            && scope != "all"
        {
            anyhow::bail!("unsupported consolidation scope: {scope}");
        }
        let report = self.store.consolidate_report()?;
        Ok(ConsolidateView {
            merged: report.merged,
            superseded: report.superseded,
            decayed: report.decayed,
        })
    }

    pub fn record_event(&self, params: RecordEventParams) -> anyhow::Result<EventView> {
        if params.session_id.trim().is_empty() {
            anyhow::bail!("invalid session_id: must not be empty");
        }
        if params.kind.trim().is_empty() {
            anyhow::bail!("invalid kind: must not be empty");
        }
        let source = params.source.as_deref().unwrap_or("mcp");
        let agent_id = params.agent_id.as_deref().unwrap_or("mcp");
        let agent_kind = params
            .agent_kind
            .as_deref()
            .unwrap_or("EXTERNAL_TOOL")
            .parse::<AgentKind>()?;
        let id = self.store.record_event_from_agent(
            agent_id,
            agent_kind,
            &params.session_id,
            &params.kind,
            &params.payload,
            source,
        )?;
        Ok(self.store.get_event(id)?.into())
    }

    pub fn should_extract_memories(
        &self,
        params: ShouldExtractMemoriesParams,
    ) -> anyhow::Result<ShouldExtractMemoriesView> {
        if params.event_threshold == 0 {
            anyhow::bail!("invalid event_threshold: must be at least 1");
        }
        Ok(ShouldExtractMemoriesView {
            should_extract: self
                .store
                .should_extract_memories(&params.session_id, params.event_threshold)?,
        })
    }

    pub fn propose_candidate_claim(
        &self,
        params: ProposeCandidateClaimParams,
    ) -> anyhow::Result<CandidateClaimView> {
        let id = self.store.propose_candidate_claim(
            params.event_id,
            &params.subject,
            &params.predicate,
            &params.object,
        )?;
        Ok(self.store.get_candidate_claim(id)?.into())
    }

    pub fn list_candidate_claims(
        &self,
        params: ListCandidateClaimsParams,
    ) -> anyhow::Result<Vec<CandidateClaimView>> {
        Ok(self
            .store
            .list_candidate_claims(params.session_id.as_deref(), params.status.as_deref())?
            .into_iter()
            .map(CandidateClaimView::from)
            .collect())
    }

    pub fn promote_candidate_claim(
        &self,
        params: PromoteCandidateClaimParams,
    ) -> anyhow::Result<ClaimView> {
        let claim_id = self.store.promote_candidate_claim(params.candidate_id)?;
        Ok(self.store.get_claim(claim_id)?.into())
    }

    pub fn reject_candidate_claim(
        &self,
        params: RejectCandidateClaimParams,
    ) -> anyhow::Result<CandidateClaimView> {
        self.store
            .reject_candidate_claim(params.candidate_id, &params.reason)?;
        Ok(self.store.get_candidate_claim(params.candidate_id)?.into())
    }
}

fn validate_hops(hops: usize) -> anyhow::Result<usize> {
    if (1..=8).contains(&hops) {
        Ok(hops)
    } else {
        anyhow::bail!("invalid hops: must be between 1 and 8")
    }
}

fn validate_top_k(top_k: usize) -> anyhow::Result<usize> {
    if (1..=100).contains(&top_k) {
        Ok(top_k)
    } else {
        anyhow::bail!("invalid top_k: must be between 1 and 100")
    }
}
