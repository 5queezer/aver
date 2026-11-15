# Predicate Diagnostics Fuzzy All Tools Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the one-tool unknown-predicate diagnostic patch with alias-backed fuzzy predicate suggestions and one shared MCP error formatter used by every tool.

**Architecture:** Keep `aver-core::Error::UnknownPredicate { name }` stable as the structured core error. Put ontology-aware diagnostics in `Store::describe_unknown_predicate` and server/tool rendering in one MCP helper, so every tool gets identical unknown-predicate messages. Treat `requires` as an accepted predicate alias for `depends_on`, and make predicate filtering alias-aware so accepted aliases work in graph tools.

**Tech Stack:** Rust, rusqlite migrations, rmcp MCP server, anyhow error chains, cargo test/clippy/fmt.

---

## File Structure

- Modify: `migrations/0086_requires_predicate_alias.sql`
  - Adds `requires` as an alias for canonical predicate `depends_on` for existing and fresh databases.
- Modify: `crates/aver-core/src/lib.rs`
  - Removes the one-off `COMMON_PREDICATE_SUGGESTIONS` mapping.
  - Adds alias-aware predicate vocabulary candidates for fuzzy diagnostics.
  - Makes `predicate_implies` and `expand_predicate_filter` resolve aliases.
- Modify: `crates/aver-core/tests/ontology_enforcement.rs`
  - Adds regression coverage for `requires` alias acceptance, fuzzy typo suggestions, and alias-aware graph predicate filters.
- Modify: `crates/aver-core/tests/migrations.rs`
  - Adds regression coverage that a fresh database contains the `requires -> depends_on` alias.
- Modify: `crates/aver-server/src/tools.rs`
  - Replaces one-off `remember_claim` error mapping with `AverTools::describe_error(&anyhow::Error)`.
- Modify: `crates/aver-server/src/mcp.rs`
  - Adds one shared `json_tool_result(&AverTools, ...)` error formatter and routes all MCP tools through it.
- Modify: `crates/aver-server/tests/tools.rs`
  - Updates the existing unknown-predicate test from `requires` to a typo that remains invalid, such as `requirse`.
- Modify: `README.md`
  - Clarifies that unknown-predicate diagnostics are tool-wide and alias/fuzzy backed.

---

### Task 1: Lock Core Alias and Fuzzy Diagnostic Behavior

**Files:**
- Modify: `crates/aver-core/tests/ontology_enforcement.rs`

- [ ] **Step 1: Add core regression tests before implementation**

Append these tests after `unknown_predicate_diagnostic_includes_available_predicates_and_suggestion` in `crates/aver-core/tests/ontology_enforcement.rs`:

```rust
#[test]
fn requires_alias_resolves_to_depends_on_for_claims_and_graph_filters() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let claim_id = store
        .add_claim_from_agent(
            "llm_agent",
            AgentKind::Llm,
            "app_server",
            "requires",
            "database",
            "llm",
        )
        .expect("requires should be accepted as an alias for depends_on");

    let claim = store.get_claim(claim_id).unwrap();
    assert_eq!(claim.predicate, "requires");
    assert!(store.predicate_implies("requires", "depends_on").unwrap());
    assert!(store.predicate_implies("requires", "relates_to").unwrap());

    let expansion = store
        .expand("app_server", 1, Some(&["depends_on"]))
        .expect("depends_on filter should include requires alias edges");
    assert!(
        expansion
            .edges
            .iter()
            .any(|edge| edge.id == claim_id && edge.predicate == "requires"),
        "depends_on graph filter should include the requires alias edge: {expansion:?}"
    );
}

#[test]
fn unknown_predicate_diagnostic_uses_fuzzy_alias_aware_suggestions() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let err = store
        .add_claim_from_agent(
            "llm_agent",
            AgentKind::Llm,
            "app_server",
            "requirse",
            "database",
            "llm",
        )
        .expect_err("misspelled aliases should still reject");

    let Error::UnknownPredicate { name } = err else {
        panic!("expected UnknownPredicate, got {err:?}");
    };
    assert_eq!(name, "requirse");

    let msg = store.describe_unknown_predicate(&name).unwrap();
    assert!(
        msg.contains("did you mean `requires` (alias for `depends_on`)?"),
        "{msg}"
    );
    assert!(msg.contains("available predicates:"), "{msg}");
    assert!(msg.contains("`depends_on`"), "{msg}");
    assert!(msg.contains("accepted aliases:"), "{msg}");
    assert!(msg.contains("`requires`"), "{msg}");
}
```

- [ ] **Step 2: Run the new core tests and verify they fail**

Run:

```bash
cargo test -q -p aver-core requires_alias_resolves_to_depends_on_for_claims_and_graph_filters unknown_predicate_diagnostic_uses_fuzzy_alias_aware_suggestions
```

Expected: FAIL. The first test rejects `requires` with `Error::UnknownPredicate`; the second test either suggests the wrong value or does not mention `requires` as an alias for `depends_on`.

- [ ] **Step 3: Commit the failing tests**

```bash
git add crates/aver-core/tests/ontology_enforcement.rs
git commit -m "test: lock predicate alias diagnostics behavior"
```

---

### Task 2: Add the `requires -> depends_on` Alias Migration

**Files:**
- Create: `migrations/0086_requires_predicate_alias.sql`
- Modify: `crates/aver-core/tests/migrations.rs`

- [ ] **Step 1: Add the migration regression test**

Append this test after `predicate_alias_created_at_must_be_positive` in `crates/aver-core/tests/migrations.rs`:

```rust
#[test]
fn fresh_database_has_requires_predicate_alias() {
    let dir = tempfile::tempdir().unwrap();
    let _store = Store::open(dir.path()).expect("open should succeed");
    drop(_store);

    let conn = rusqlite::Connection::open(dir.path().join("db.sqlite")).unwrap();
    let canonical: String = conn
        .query_row(
            "SELECT predicate_types.name
               FROM predicate_alias
               JOIN predicate_types ON predicate_types.id = predicate_alias.predicate_id
              WHERE predicate_alias.alias = 'requires'",
            [],
            |row| row.get(0),
        )
        .expect("requires alias should exist");

    assert_eq!(canonical, "depends_on");
}
```

- [ ] **Step 2: Run the migration test and verify it fails**

Run:

```bash
cargo test -q -p aver-core fresh_database_has_requires_predicate_alias
```

Expected: FAIL with `requires alias should exist`.

- [ ] **Step 3: Create the migration**

Create `migrations/0086_requires_predicate_alias.sql` with exactly:

```sql
-- Add a semantic predicate alias for model-authored dependency claims.
-- `requires` means the same relationship as canonical `depends_on`, but
-- keeping it as an alias preserves source/log fidelity for callers that emit
-- natural-language relation names.
INSERT OR IGNORE INTO predicate_alias (alias, predicate_id, created_at, note)
SELECT 'requires', id, strftime('%s','now'), 'semantic alias for depends_on'
  FROM predicate_types
 WHERE name = 'depends_on';
```

- [ ] **Step 4: Run the migration test and verify it passes**

Run:

```bash
cargo test -q -p aver-core fresh_database_has_requires_predicate_alias
```

Expected: PASS.

- [ ] **Step 5: Commit the migration**

```bash
git add migrations/0086_requires_predicate_alias.sql crates/aver-core/tests/migrations.rs
git commit -m "feat: add requires predicate alias"
```

---

### Task 3: Replace One-Off Suggestions with Alias-Aware Fuzzy Suggestions

**Files:**
- Modify: `crates/aver-core/src/lib.rs:226-351`

- [ ] **Step 1: Replace the current suggestion data structures**

In `crates/aver-core/src/lib.rs`, replace the `COMMON_PREDICATE_SUGGESTIONS` constant and `suggest_unknown_predicate` function with this alias-aware candidate model:

```rust
const UNKNOWN_PREDICATE_LIST_LIMIT: usize = 32;

#[derive(Debug, Clone, Eq, PartialEq)]
struct PredicateCandidate {
    accepted: String,
    canonical: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct PredicateSuggestion {
    accepted: String,
    canonical: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct PredicateVocabulary {
    predicates: Vec<String>,
    aliases: Vec<String>,
    candidates: Vec<PredicateCandidate>,
}

fn normalize_predicate_name(name: &str) -> String {
    name.trim()
        .chars()
        .map(|ch| match ch {
            '-' | ' ' => '_',
            _ => ch.to_ascii_lowercase(),
        })
        .collect()
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut current = vec![0; right_chars.len() + 1];
    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let deletion = previous[right_index + 1] + 1;
            let insertion = current[right_index] + 1;
            let substitution = previous[right_index] + usize::from(left_char != *right_char);
            current[right_index + 1] = deletion.min(insertion).min(substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right_chars.len()]
}

fn suggest_unknown_predicate(
    name: &str,
    candidates: &[PredicateCandidate],
) -> Option<PredicateSuggestion> {
    let normalized = normalize_predicate_name(name);
    let mut best: Option<(&PredicateCandidate, usize)> = None;

    for candidate in candidates {
        let candidate_normalized = normalize_predicate_name(&candidate.accepted);
        let distance = edit_distance(&normalized, &candidate_normalized);
        let is_better = match best {
            None => true,
            Some((best_candidate, best_distance)) => {
                distance < best_distance
                    || (distance == best_distance
                        && candidate.accepted.len() < best_candidate.accepted.len())
                    || (distance == best_distance
                        && candidate.accepted.len() == best_candidate.accepted.len()
                        && candidate.accepted < best_candidate.accepted)
            }
        };
        if is_better {
            best = Some((candidate, distance));
        }
    }

    best.and_then(|(candidate, distance)| {
        let threshold = match normalized.len().max(candidate.accepted.len()) {
            0..=4 => 1,
            5..=8 => 2,
            _ => 3,
        };
        (distance <= threshold).then(|| PredicateSuggestion {
            accepted: candidate.accepted.clone(),
            canonical: candidate.canonical.clone(),
        })
    })
}
```

- [ ] **Step 2: Replace the formatter with alias-aware suggestion wording**

Replace `format_unknown_predicate` with:

```rust
fn format_unknown_predicate(
    name: &str,
    suggestion: Option<&PredicateSuggestion>,
    available_predicates: &[String],
    available_aliases: &[String],
) -> String {
    let mut message = format!("unknown predicate: {name} (not in predicate_types or predicate_alias).");
    if let Some(suggestion) = suggestion {
        match suggestion.canonical.as_deref() {
            Some(canonical) if canonical != suggestion.accepted => message.push_str(&format!(
                " did you mean `{}` (alias for `{canonical}`)?",
                suggestion.accepted
            )),
            _ => message.push_str(&format!(" did you mean `{}`?", suggestion.accepted)),
        }
    }
    if !available_predicates.is_empty() {
        message.push_str(" available predicates: ");
        message.push_str(&format_available_values(available_predicates));
        message.push('.');
    }
    if !available_aliases.is_empty() {
        message.push_str(" accepted aliases: ");
        message.push_str(&format_available_values(available_aliases));
        message.push('.');
    }
    message
}
```

- [ ] **Step 3: Update `Store::describe_unknown_predicate` and vocabulary queries**

Replace the existing `describe_unknown_predicate`, `predicate_vocabulary`, and `query_string_column` block with:

```rust
    /// Build a user-facing diagnostic for an unknown predicate using the
    /// current runtime ontology tables.
    pub fn describe_unknown_predicate(&self, predicate: &str) -> Result<String, Error> {
        let vocabulary = self.predicate_vocabulary()?;
        let suggestion = suggest_unknown_predicate(predicate, &vocabulary.candidates);
        Ok(format_unknown_predicate(
            predicate,
            suggestion.as_ref(),
            &vocabulary.predicates,
            &vocabulary.aliases,
        ))
    }
```

Then replace the existing private `predicate_vocabulary` and `query_string_column` methods with:

```rust
    fn predicate_vocabulary(&self) -> Result<PredicateVocabulary, Error> {
        let predicates =
            self.query_string_column("SELECT name FROM predicate_types ORDER BY name")?;
        let alias_rows = self.query_alias_rows()?;
        let aliases = alias_rows
            .iter()
            .map(|(alias, _canonical)| alias.clone())
            .collect::<Vec<_>>();
        let mut candidates = predicates
            .iter()
            .map(|predicate| PredicateCandidate {
                accepted: predicate.clone(),
                canonical: None,
            })
            .collect::<Vec<_>>();
        candidates.extend(
            alias_rows
                .iter()
                .map(|(alias, canonical)| PredicateCandidate {
                    accepted: alias.clone(),
                    canonical: Some(canonical.clone()),
                }),
        );
        Ok(PredicateVocabulary {
            predicates,
            aliases,
            candidates,
        })
    }

    fn query_string_column(&self, sql: &str) -> Result<Vec<String>, Error> {
        let mut statement = self.conn.prepare(sql)?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Error::Sqlite)
    }

    fn query_alias_rows(&self) -> Result<Vec<(String, String)>, Error> {
        let mut statement = self.conn.prepare(
            "SELECT predicate_alias.alias, predicate_types.name
               FROM predicate_alias
               JOIN predicate_types ON predicate_types.id = predicate_alias.predicate_id
              ORDER BY predicate_alias.alias",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Error::Sqlite)
    }
```

- [ ] **Step 4: Run the fuzzy diagnostic test and verify it passes**

Run:

```bash
cargo test -q -p aver-core unknown_predicate_diagnostic_uses_fuzzy_alias_aware_suggestions
```

Expected: PASS.

- [ ] **Step 5: Commit the fuzzy diagnostic implementation**

```bash
git add crates/aver-core/src/lib.rs crates/aver-core/tests/ontology_enforcement.rs
git commit -m "fix: use alias-aware fuzzy predicate suggestions"
```

---

### Task 4: Make Predicate Filters Alias-Aware

**Files:**
- Modify: `crates/aver-core/src/lib.rs:470-555`

- [ ] **Step 1: Add canonical predicate resolution**

Add this method immediately before `pub fn predicate_implies` in `impl Store`:

```rust
    fn canonical_predicate_name(&self, predicate: &str) -> Result<Option<String>, Error> {
        if self.predicate_type_id(predicate)?.is_some() {
            return Ok(Some(predicate.to_string()));
        }
        self.conn
            .query_row(
                "SELECT predicate_types.name
                   FROM predicate_alias
                   JOIN predicate_types ON predicate_types.id = predicate_alias.predicate_id
                  WHERE predicate_alias.alias = ?1",
                [predicate],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::Sqlite)
    }
```

- [ ] **Step 2: Replace `predicate_implies` with alias-aware logic**

Replace the existing `pub fn predicate_implies` with:

```rust
    pub fn predicate_implies(&self, predicate: &str, ancestor: &str) -> Result<bool, Error> {
        validate_claim_field("predicate", predicate)?;
        validate_claim_field("predicate", ancestor)?;
        let Some(predicate_name) = self.canonical_predicate_name(predicate)? else {
            return Ok(false);
        };
        let Some(ancestor_name) = self.canonical_predicate_name(ancestor)? else {
            return Ok(false);
        };
        if predicate_name == ancestor_name {
            return Ok(true);
        }
        let Some(predicate_id) = self.predicate_type_id(&predicate_name)? else {
            return Ok(false);
        };
        let Some(ancestor_id) = self.predicate_type_id(&ancestor_name)? else {
            return Ok(false);
        };
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM predicate_closure WHERE child_id = ?1 AND ancestor_id = ?2",
                params![predicate_id, ancestor_id],
                |_| Ok(()),
            )
            .is_ok())
    }
```

- [ ] **Step 3: Replace `expand_predicate_filter` with alias-aware descendant expansion**

Replace the existing `fn expand_predicate_filter` with:

```rust
    fn expand_predicate_filter(&self, predicates: &[&str]) -> Result<HashSet<String>, Error> {
        let mut allowed = HashSet::new();
        for predicate in predicates {
            allowed.insert((*predicate).to_string());
            let Some(canonical) = self.canonical_predicate_name(predicate)? else {
                continue;
            };
            allowed.insert(canonical.clone());

            let mut stmt = self.conn.prepare(
                "SELECT child.name
                   FROM predicate_types child
                   JOIN predicate_closure closure ON closure.child_id = child.id
                   JOIN predicate_types ancestor ON ancestor.id = closure.ancestor_id
                  WHERE ancestor.name = ?1
                  ORDER BY child.id",
            )?;
            let rows = stmt.query_map([canonical.as_str()], |row| row.get::<_, String>(0))?;
            let child_names = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);

            for child_name in child_names {
                allowed.insert(child_name.clone());
                let mut alias_stmt = self.conn.prepare(
                    "SELECT alias
                       FROM predicate_alias
                       JOIN predicate_types ON predicate_types.id = predicate_alias.predicate_id
                      WHERE predicate_types.name = ?1
                      ORDER BY alias",
                )?;
                let alias_rows = alias_stmt.query_map([child_name.as_str()], |row| {
                    row.get::<_, String>(0)
                })?;
                for alias in alias_rows {
                    allowed.insert(alias?);
                }
            }
        }
        Ok(allowed)
    }
```

- [ ] **Step 4: Run the alias graph-filter test and verify it passes**

Run:

```bash
cargo test -q -p aver-core requires_alias_resolves_to_depends_on_for_claims_and_graph_filters
```

Expected: PASS.

- [ ] **Step 5: Commit the alias-aware filter implementation**

```bash
git add crates/aver-core/src/lib.rs crates/aver-core/tests/ontology_enforcement.rs
git commit -m "fix: make predicate filters alias-aware"
```

---

### Task 5: Move Unknown-Predicate Error Rendering to a Shared Tool Boundary

**Files:**
- Modify: `crates/aver-server/src/tools.rs:1-20,523-570`
- Modify: `crates/aver-server/tests/tools.rs:50-72`

- [ ] **Step 1: Add a tool-layer error describer**

In `crates/aver-server/src/tools.rs`, keep the `Error` import and replace the current private `fn core_error(&self, err: Error) -> anyhow::Error` with:

```rust
    pub fn describe_error(&self, err: &anyhow::Error) -> String {
        if let Some(Error::UnknownPredicate { name }) = err.downcast_ref::<Error>() {
            return self
                .store
                .describe_unknown_predicate(name)
                .unwrap_or_else(|vocab_err| {
                    format!(
                        "unknown predicate: {name} (not in predicate_types or predicate_alias). failed to load available predicates: {vocab_err}"
                    )
                });
        }
        err.to_string()
    }
```

- [ ] **Step 2: Preserve the original error chain in `remember_claim`**

In `crates/aver-server/src/tools.rs`, replace the final `let id = ...` block in `remember_claim` with:

```rust
        let id = self.store.add_claim_from_agent_with_scope(
            agent_id,
            agent_kind,
            &params.subject,
            &params.predicate,
            &params.object,
            source,
            scope,
        )?;
```

This removes `.map_err(|err| self.core_error(err))?` so `anyhow::Error::downcast_ref::<aver_core::Error>()` still finds the original `UnknownPredicate`.

- [ ] **Step 3: Update the direct tool test to use a still-invalid typo**

In `crates/aver-server/tests/tools.rs`, rename `remember_claim_unknown_predicate_error_lists_vocabulary_and_suggestion` to `remember_claim_unknown_predicate_typo_lists_vocabulary_and_suggestion`, and change the predicate from `requires` to `requirse`:

```rust
#[test]
fn remember_claim_unknown_predicate_typo_lists_vocabulary_and_suggestion() {
    let dir = tempfile::tempdir().unwrap();
    let tools = AverTools::open(dir.path()).unwrap();

    let err = tools
        .remember_claim(RememberClaimParams {
            subject: "PaymentGateway".to_string(),
            predicate: "requirse".to_string(),
            object: "StripeSDK".to_string(),
            source: Some("mcp-test".to_string()),
            agent_id: Some("llm_agent".to_string()),
            agent_kind: Some("LLM".to_string()),
            scope: None,
        })
        .expect_err("LLM claims with misspelled predicates should explain the vocabulary");

    let msg = tools.describe_error(&err);
    assert!(msg.contains("unknown predicate: requirse"), "{msg}");
    assert!(
        msg.contains("did you mean `requires` (alias for `depends_on`)?"),
        "{msg}"
    );
    assert!(msg.contains("available predicates:"), "{msg}");
    assert!(msg.contains("`depends_on`"), "{msg}");
    assert!(msg.contains("accepted aliases:"), "{msg}");
    assert!(msg.contains("`requires`"), "{msg}");
}
```

- [ ] **Step 4: Run the direct tool test and verify it passes**

Run:

```bash
cargo test -q -p aver-server remember_claim_unknown_predicate_typo_lists_vocabulary_and_suggestion
```

Expected: PASS.

- [ ] **Step 5: Commit the tool-layer error describer**

```bash
git add crates/aver-server/src/tools.rs crates/aver-server/tests/tools.rs
git commit -m "refactor: preserve predicate errors for tool diagnostics"
```

---

### Task 6: Route Every MCP Tool Through the Shared Error Formatter

**Files:**
- Modify: `crates/aver-server/src/mcp.rs:1-720`

- [ ] **Step 1: Add `MutexGuard` to the imports**

At the top of `crates/aver-server/src/mcp.rs`, change:

```rust
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
```

to:

```rust
use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};
```

- [ ] **Step 2: Add a shared lock helper to `impl AverMcpService`**

Inside `impl AverMcpService`, immediately after `pub fn open(...)`, add:

```rust
    fn lock_tools(&self) -> Result<MutexGuard<'_, AverTools>, McpError> {
        self.tools.lock().map_err(|err| {
            McpError::new(
                ErrorCode::INTERNAL_ERROR,
                format!("memory tool lock poisoned: {err}"),
                None,
            )
        })
    }
```

- [ ] **Step 3: Replace `json_tool_result` with the shared formatter**

Replace the existing free function `fn json_tool_result<T: serde::Serialize>(result: anyhow::Result<T>, tool_name: &str)` with:

```rust
fn json_tool_result<T: serde::Serialize>(
    tools: &AverTools,
    result: anyhow::Result<T>,
    tool_name: &str,
) -> Result<CallToolResult, McpError> {
    match result {
        Ok(value) => Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&value).unwrap_or_default(),
        )])),
        Err(err) => Err(McpError::new(
            ErrorCode::INTERNAL_ERROR,
            format!("{tool_name} failed: {}", tools.describe_error(&err)),
            None,
        )),
    }
}
```

- [ ] **Step 4: Update `remember_claim` to use the shared formatter**

Replace the body after `require_scope(&ctx, "remember_claim")?;` with:

```rust
        let tools = self.lock_tools()?;
        let result = tools.remember_claim(CoreRememberClaimParams {
            subject: params.subject,
            predicate: params.predicate,
            object: params.object,
            source: params.source,
            agent_id: params.agent_id,
            agent_kind: params.agent_kind,
            scope: params.scope.or_else(|| Some(request_scope(&ctx).scope)),
        });
        json_tool_result(&tools, result, "remember_claim")
```

- [ ] **Step 5: Update `add_triple` to use the shared formatter**

Replace the lock/result block in `add_triple` with:

```rust
        let tools = self.lock_tools()?;
        let result = tools.add_triple(params);
        json_tool_result(&tools, result, "add_triple")
```

- [ ] **Step 6: Update every remaining `json_tool_result` handler with the same lock pattern**

For each tool handler below, replace the repeated `self.tools.lock().map_err(...)?` block with `let tools = self.lock_tools()?;`, call the same `tools.<method>(...)`, and pass `&tools` to `json_tool_result`:

```rust
// expand
let tools = self.lock_tools()?;
let result = tools.expand(params);
json_tool_result(&tools, result, "expand")

// contradict
let tools = self.lock_tools()?;
let result = tools.contradict(params);
json_tool_result(&tools, result, "contradict")

// consolidate
let tools = self.lock_tools()?;
let result = tools.consolidate(params);
json_tool_result(&tools, result, "consolidate")

// record_event
let tools = self.lock_tools()?;
let result = tools.record_event(params);
json_tool_result(&tools, result, "record_event")

// should_extract_memories
let tools = self.lock_tools()?;
let result = tools.should_extract_memories(params);
json_tool_result(&tools, result, "should_extract_memories")

// propose_candidate_claim
let tools = self.lock_tools()?;
let result = tools.propose_candidate_claim(params);
json_tool_result(&tools, result, "propose_candidate_claim")

// list_candidate_claims
let tools = self.lock_tools()?;
let result = tools.list_candidate_claims(params);
json_tool_result(&tools, result, "list_candidate_claims")

// promote_candidate_claim
let tools = self.lock_tools()?;
let result = tools.promote_candidate_claim(params);
json_tool_result(&tools, result, "promote_candidate_claim")

// reject_candidate_claim
let tools = self.lock_tools()?;
let result = tools.reject_candidate_claim(params);
json_tool_result(&tools, result, "reject_candidate_claim")

// record_observation
let tools = self.lock_tools()?;
let result = tools.record_observation(params);
json_tool_result(&tools, result, "record_observation")

// recall_observation
let tools = self.lock_tools()?;
let result = tools.recall_observation(params);
json_tool_result(&tools, result, "recall_observation")

// observation_coverage
let tools = self.lock_tools()?;
let result = tools.observation_coverage(params);
json_tool_result(&tools, result, "observation_coverage")

// assemble_compaction_summary
let tools = self.lock_tools()?;
let result = tools.assemble_compaction_summary(params);
json_tool_result(&tools, result, "assemble_compaction_summary")

// add_vector_chunk
let tools = self.lock_tools()?;
let result = tools.add_vector_chunk(params);
json_tool_result(&tools, result, "add_vector_chunk")

// retire_claim
let tools = self.lock_tools()?;
let result = tools.retire_claim(params);
json_tool_result(&tools, result, "retire_claim")
```

Keep the existing parameter defaulting logic before each lock block. Do not move scope mutation after the tool call.

- [ ] **Step 7: Update `recall` to use the shared formatter**

Replace the custom `match result` block in `recall` with:

```rust
        let tools = self.lock_tools()?;
        let result = tools.recall({
            let resolved = request_scope(&ctx);
            let walk = params.scope_walk.clone().or_else(|| {
                if params.scope.is_none() {
                    Some(resolved.default_walk.as_str().to_string())
                } else {
                    None
                }
            });
            CoreRecallParams {
                query: params.query,
                alpha: params.alpha,
                hops: params.hops,
                top_k: Some(params.top_k),
                scope: params.scope.or(Some(resolved.scope)),
                scope_walk: walk,
                agent_id: None,
                agent_kind: None,
                predicate: None,
                predicate_walk: None,
                min_confidence: None,
                status: None,
            }
        });
        json_tool_result(&tools, result, "recall")
```

- [ ] **Step 8: Add a private MCP formatter unit test**

Append this module near the bottom of `crates/aver-server/src/mcp.rs`, before `#[tool_handler] impl ServerHandler for AverMcpService`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_tool_result_enriches_unknown_predicate_for_any_tool_name() {
        let dir = tempfile::tempdir().unwrap();
        let tools = AverTools::open(dir.path()).unwrap();

        let err = json_tool_result::<serde_json::Value>(
            &tools,
            Err(aver_core::Error::UnknownPredicate {
                name: "requirse".to_string(),
            }
            .into()),
            "add_triple",
        )
        .expect_err("unknown predicate should become an MCP error");

        let msg = err.message.to_string();
        assert!(msg.contains("add_triple failed: unknown predicate: requirse"), "{msg}");
        assert!(
            msg.contains("did you mean `requires` (alias for `depends_on`)?"),
            "{msg}"
        );
        assert!(msg.contains("available predicates:"), "{msg}");
        assert!(msg.contains("accepted aliases:"), "{msg}");
    }
}
```

- [ ] **Step 9: Run MCP formatter tests and server tests**

Run:

```bash
cargo test -q -p aver-server json_tool_result_enriches_unknown_predicate_for_any_tool_name remember_claim_unknown_predicate_typo_lists_vocabulary_and_suggestion
```

Expected: PASS.

- [ ] **Step 10: Commit the MCP-wide formatter**

```bash
git add crates/aver-server/src/mcp.rs crates/aver-server/src/tools.rs crates/aver-server/tests/tools.rs
git commit -m "fix: use shared MCP error diagnostics"
```

---

### Task 7: Documentation and Full Verification

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update README ontology reasoner wording**

Replace the ontology reasoner bullet text that mentions unknown predicate diagnostics with:

```markdown
- **Ontology reasoner** — ADR-0010 entity and predicate hierarchies are seeded on open, materialized into closure tables, and used by graph expansion and path predicate filters so abstract filters such as `depends_on` also match descendant predicates like `calls`, `imports`, and accepted aliases such as `requires`; MCP/tool-facing diagnostics for unknown non-user predicates use alias-aware fuzzy suggestions plus the current predicate/alias vocabulary.
```

- [ ] **Step 2: Run formatting**

Run:

```bash
cargo fmt --all
```

Expected: no output.

- [ ] **Step 3: Run targeted tests**

Run:

```bash
cargo test -q -p aver-core requires_alias_resolves_to_depends_on_for_claims_and_graph_filters unknown_predicate_diagnostic_uses_fuzzy_alias_aware_suggestions fresh_database_has_requires_predicate_alias
cargo test -q -p aver-server json_tool_result_enriches_unknown_predicate_for_any_tool_name remember_claim_unknown_predicate_typo_lists_vocabulary_and_suggestion
```

Expected: PASS.

- [ ] **Step 4: Run quality gates**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Expected: all commands pass.

- [ ] **Step 5: Commit docs and verification-ready state**

```bash
git add README.md
git commit -m "docs: describe alias-aware predicate diagnostics"
```

- [ ] **Step 6: Add a workflow-learning git note to the final commit**

Run after the final commit hash is known:

```bash
git notes add -m "problem: Unknown-predicate diagnostics were one-tool and had a one-off semantic suggestion
action: Added requires as a predicate alias, made fuzzy diagnostics alias-aware, and routed all MCP tool errors through one formatter
failed_paths:
  - One-off remember_claim enrichment did not scale to add_triple or future predicate-writing tools
verification:
  - cargo fmt --all -- --check
  - cargo clippy --all-targets --all-features -- -D warnings
  - cargo test --all-features
workflow_learning: Keep core error types stable, add semantic aliases through ontology data, and centralize user-facing MCP diagnostics at the tool boundary
related_files:
  - migrations/0086_requires_predicate_alias.sql
  - crates/aver-core/src/lib.rs
  - crates/aver-server/src/tools.rs
  - crates/aver-server/src/mcp.rs
  - crates/aver-core/tests/ontology_enforcement.rs
  - crates/aver-server/tests/tools.rs
  - README.md" HEAD
```

---

## Self-Review

**Spec coverage:**
- Fuzzy search: Task 3 replaces the one-off mapping with fuzzy matching over canonical predicates and aliases.
- Not just one tool: Task 6 routes every MCP handler through one formatter.
- `requires` behavior: Task 2 adds an ontology alias, Task 4 makes graph filters alias-aware, and Task 1 locks the behavior.
- API stability: `Error::UnknownPredicate { name }` remains unchanged.
- Tests first: Tasks 1, 2, and 6 add failing tests before implementation.

**Placeholder scan:** No placeholders, no open-ended edge-case language, and all commands have expected results.

**Type consistency:** The plan uses `PredicateCandidate`, `PredicateSuggestion`, `PredicateVocabulary`, `AverTools::describe_error`, and `json_tool_result(&AverTools, ...)` consistently across tasks.
