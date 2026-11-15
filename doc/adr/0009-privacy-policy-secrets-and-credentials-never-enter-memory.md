# 9. Privacy policy: secrets and credentials never enter memory

Date: 2026-05-06

## Status

Accepted

## Context

A coding agent observes everything: file contents, environment variables, command output, git diffs, browser tabs, chat history. The default "remember everything observable" pattern is a security incident waiting to happen — secrets that leave only ephemeral process memory today would become persistent, queryable, syncable graph claims.

The book gestures at provenance and confidence [ch.90, 81] but says little about *exclusion*. This ADR fills the gap explicitly because the omission is dangerous, not benign.

The user's environment makes the threat concrete: API keys live in `~/.secrets.d/` and are loaded per-project via `direnv`. A naive memory layer scanning environment variables or shell history would harvest every key on disk.

## Decision

A pre-write filter rejects content matching any of the following before it touches the episodic log:

### Detector cascade

1. **High-entropy strings**: any token with Shannon entropy > 4.5 bits/char and length > 20 characters is treated as a potential secret.
2. **Regex catalog** (non-exhaustive starter set):
   - AWS access key (`AKIA[0-9A-Z]{16}`), secret key,
   - GitHub PAT (`ghp_…`), fine-grained PAT (`github_pat_…`), GitHub Actions token,
   - JWT (`eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}`),
   - Slack bot/user/webhook tokens,
   - OpenAI / Anthropic / Stripe key prefixes (`sk-`, `sk-ant-`, `sk_live_`),
   - SSH private key headers, PGP private blocks, generic `BEGIN PRIVATE KEY`.
3. **Path-based exclusions**: any content read from
   - `~/.secrets.d/**`, `~/.ssh/**`, `~/.aws/credentials`, `~/.config/**`,
   - any file matched by the project's `.gitignore`,
   - any path containing `.env`, `id_rsa`, `id_ed25519`, `*.pem`, `*.key`.
4. **Explicit markers**: lines containing `# memory:ignore` or files containing a top-of-file `<!-- memory:ignore -->` comment skip extraction entirely.

### Filter placement

The filter runs **before episodic write**, not before consolidation. Once a secret hits the log, it's already persisted — even if consolidation never promotes it to the graph, recovery requires log rewrite. Treat the log as immutable once written.

### Telemetry without leaks

When the filter rejects content, increment a metrics counter `memory.filter.rejected{reason="entropy"|"regex:aws"|"path:secrets-dir"|...}` — but **never** log the offending content or its hash. Hashes leak structure for short secrets.

### Override

Explicit `memory.add_triple(...)` calls with content that matches a detector are still rejected. The agent cannot override the filter. The user can, by editing the detector config — a deliberate friction point.

## Consequences

- (+) Hard floor against the most common leak class (env vars, key files, tokens in tool output).
- (+) Telemetry detects exfil attempts without recording them.
- (+) Aligns with the user's existing per-secret file convention (`~/.secrets.d/<name>`).
- (−) Filter is a single point of failure: a bypass bug = a leak. Treat it as security-critical code, version-pin detector updates, run the regex catalog against a corpus of synthetic secrets in CI.
- (−) False positives drop legitimate memories silently (e.g., a high-entropy hash that's actually a git SHA). The metrics counter is the only signal — needs alerting on rejection rate.
- (−) Detector list is a moving target as new credential formats appear; treat it as a dependency to update, not a one-shot.
