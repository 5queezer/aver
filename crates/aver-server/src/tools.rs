use std::path::Path;

use aver_core::{AgentKind, Claim, Store};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
pub struct RecallParams {
    pub query: String,
    #[serde(default)]
    pub top_k: Option<usize>,
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

    pub fn recall(&self, params: RecallParams) -> anyhow::Result<Vec<ClaimView>> {
        let top_k = params.top_k.unwrap_or(5).clamp(1, 100);
        let mut claims = self.store.recall_text(&params.query)?;
        claims.truncate(top_k);
        Ok(claims.into_iter().map(ClaimView::from).collect())
    }
}
