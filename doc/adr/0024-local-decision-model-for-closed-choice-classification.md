# 24. Local decision model for closed-choice classification

Date: 2026-10-24

## Status

Proposed

## Context

Several places in Aver make a pick-one-of-N decision with a hand-written
heuristic:

- `retrieval::HybridWeights::for_query` classifies query intent by substring
  lists (ADR-0004).
- ADR-0014 triggers are deterministic (explicit remember, session end,
  commit, event-count threshold) and candidate promotion is manual. The ADR
  states: "LLM-based trigger classifiers may be added later, but must not be
  required for offline tests."
- Unknown predicates fall back to alias-aware fuzzy suggestions against the
  ontology vocabulary (ADR-0010, ADR-0018).
- Extractor output and most candidates land in scope `global` because no
  component decides otherwise (ADR-0021; design review 2026-10-17).
- `consolidate_report` acts only on exact `(subject, predicate, object)`
  matches; near-duplicates and paraphrased contradictions are never seen.
- The BEAM eval runner (ADR-0012) uses a generative Ollama model as rubric
  judge and parses a numeric score out of free text.

Small "decision models" now exist that take a state, a question and a closed
list of 2–24 options and return exactly one option. Together AI's
`Tev1-4B-experimental` (a Qwen3.5-4B fine-tune, weights on Hugging Face,
`ollama pull tev1:4b`) is the first; the data recipe is public and training
cost about $17, so a project-specific fine-tune is realistic. These models do
not generate text, so they cannot hallucinate a triple or leak a secret into
memory. They can only pick an option that the caller already enumerated.

That constraint matches ADR-0007's spirit (deterministic where a parser
exists, model only at the edge) and ADR-0003's provenance discipline better
than a general LLM does. It also fits Aver's local-first posture (ADR-0006):
the model runs in the same Ollama instance that already serves embeddings.

## Decision

Add a `DecisionClient` abstraction to `aver-core` and use it only for
closed-choice decisions where the option set is enumerated by Aver.

### Abstraction

```rust
pub struct Decision<'a> {
    pub state: &'a str,          // context the model reads
    pub question: &'a str,       // what is being decided
    pub options: &'a [&'a str],  // 2..=24 candidate answers, caller-enumerated
    pub descriptions: Option<&'a [&'a str]>, // one per option, sent as `criteria`
}

pub struct Choice {
    pub index: usize,            // always a valid index into `options`
    pub model: String,           // model identifier, e.g. "tev1:4b"
    pub confidence: Option<f64>, // reported by the model when available
}

pub trait DecisionClient {
    fn decide(&self, decision: &Decision<'_>) -> Result<Choice, DecisionError>;
}
```

- Lives next to `EmbeddingClient` in `aver-core` (new module `decision.rs`).
- `OllamaDecisionClient` behind the existing `ollama` feature, using the
  `ureq` client already present. It posts to Ollama's `/v1/systemone`
  endpoint (TypeSafe's Jev API) with one `choice` question. Ollama documents
  that Tev1 "tends to reply in prose" over `/api/chat`, so the chat endpoint
  is not used. Model name and base URL are configuration, never literals (the
  design review flagged `"nomic-embed-text"` for the same reason).
- `NoopDecisionClient` (always returns an error or a configured default) and
  `MockDecisionClient` (scripted answers) ship for offline tests. No test may
  require a running Ollama.
- Any model response that does not map to an option index is a
  `DecisionError::InvalidChoice`; callers fall back to the existing
  deterministic behaviour.

### Prompt discipline

Every seam sends a description per option (the systemone `criteria` value)
while the short option key stays what `Choice.index` refers to. Live calls
against `tev1:4b` on 2026-10-24 showed bare keys flip answers (predicate
`needs`: `none` 0.70 with keys, `depends_on` 0.99 with descriptions). A
model choice may influence a write only when its confidence is at least the
floor `AVER_DECISION_MIN_CONFIDENCE` (default 0.5, range 0.0..=1.0); below
it the seam takes its deterministic fallback (triage holds, predicates use
fuzzy suggestions, scope keeps the caller default, the trigger keeps the
deterministic decision). A client that reports no confidence meets the floor.

### Provenance and replay

- A write influenced by a model decision records provenance `INFERRED` and
  confidence `0.45` (ADR-0003 policy table, "single-source LLM inference").
  The JSONL line carries the chosen option and the model identifier.
- Replay never calls the model. Replay reads the recorded choice. This keeps
  ADR-0005 replay deterministic and keeps the log the source of truth.
- A decision client is never consulted on the hot recall path by default. It
  runs at write time or between sessions (consolidation, triggers).

### Where it is used

| Seam | Question | Options | Fallback when unavailable |
|------|----------|---------|---------------------------|
| ADR-0014 triggers | Should this event batch run extraction? | `extract`, `skip` | existing deterministic triggers |
| ADR-0014 candidates | What happens to this staged claim? | `promote`, `reject`, `hold` | manual promotion |
| ADR-0010/0018 predicates | Which canonical predicate matches this input? | ontology vocabulary + `none` | fuzzy suggestions |
| ADR-0021 scope | Which scope does this candidate belong to? | `global`, `project`, `session` | current default |
| ADR-0012 eval judge | Does the answer satisfy this rubric item? | `yes`, `partial`, `no` | generative judge |

Consolidation pair triage (`duplicate`, `contradiction`, `unrelated`) is a
candidate seam but is deferred: it needs a near-duplicate candidate generator
first, and a model must only write contradiction records, never supersede.

HybridRAG intent classification (ADR-0004) is deferred until the MCP recall
path stops discarding `alpha` (design review 2026-10-17, "Trace of one
recall call"). Classifying intent for a weight nobody reads changes nothing.

### Where it is not used

- **Prose extraction.** Producing triples requires generation. The chat
  heuristic extractor, or a generative model per ADR-0007, keeps that job.
- **Privacy gate (ADR-0009).** Stays deterministic. A decision model may add a
  `suspicious` flag that blocks a write. It may never clear content.
- **Observation summaries (ADR-0016).** Generation.

## Consequences

- (+) Every model-influenced decision is bounded to an option Aver enumerated.
  No model output enters memory as free text.
- (+) One trait, one Ollama transport, one mock. Offline tests stay offline.
- (+) The seams listed above stop being hardcoded without becoming
  non-deterministic on replay.
- (−) A 4B model on CPU costs hundreds of milliseconds per call. Callers that
  loop over many items must batch between sessions.
- (−) `Tev1-4B-experimental` is experimental: 88.0% on Together's own
  development set, weights licence listed as unspecified by at least one
  aggregator. The abstraction must not depend on this one model. Check the
  model card before shipping a default.
- (−) A second Ollama dependency widens the surface the design review wants
  moved behind a trait at the edge. This ADR puts the new one behind a trait
  from the start; moving `EmbeddingClient` to the same edge is separate work.

## References

- Together AI, Tev1-4B-experimental model card:
  https://huggingface.co/togethercomputer/Tev1-4B-experimental
- Together AI announcement (data recipe, fine-tuning tutorial):
  https://x.com/togethercompute/status/2102882216950763814
- Ollama library entry: https://ollama.com/library/tev1
- Design review 2026-10-17: `doc/design-review-2026-10-17.md`
