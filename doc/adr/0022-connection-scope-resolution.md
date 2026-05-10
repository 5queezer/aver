# 22. Connection-scope resolution

Date: 2026-05-10

## Status

Proposed

## Context

ADR-0021 introduces `scope` as a first-class column on `claims`, `events`, and
`observations`, with a hierarchical path convention (`global`, `proj/<slug>`,
`proj/<slug>/branch/<name>`, `session/<id>`). It deliberately defers the
question of *how a client picks the scope* to a follow-on ADR. This is that
ADR.

The motivating asymmetry is between two kinds of clients:

1. Clients that **know their scope** because they were spawned in a specific
   working directory or against a specific repository (Claude Code, Codex CLI,
   any MCP harness running in a project).
2. Clients that **operate cross-cutting** — consolidation passes, audit
   tooling, the eval harness in `crates/aver-eval` — and legitimately need to
   read or write across scopes.

ADR-0021 already serves case (2): omit `scope`, get `'global'` reads with
`scope_walk='any'`. That keeps today's behavior verbatim for cross-cutting
tools.

Case (1) is the problem this ADR addresses. Today, Claude Code's MCP config
(`~/.claude.json`) points at `http://127.0.0.1:3317/mcp` with no per-project
state. Every Claude session — across every repo — talks to the same HTTP
endpoint with no signal that distinguishes one project's writes from another.
Adding `scope` as a per-tool parameter is necessary but not sufficient: most
agent harnesses will not thread it through their tool calls reliably, and a
project memory store that depends on every LLM call remembering to set a
parameter has lost.

The constraint is that the aver-server (`crates/aver-server/src/http.rs`) is
an HTTP MCP, not an stdio child process spawned per-project. The transport is
shared. Therefore scope resolution must happen at one of these boundaries:

| Boundary                          | What it can know                                                    |
|-----------------------------------|---------------------------------------------------------------------|
| MCP tool parameter                | Whatever the LLM passes — unreliable                                |
| HTTP header on every request      | Whatever the client decides to send — reliable if client cooperates |
| Per-connection state at handshake | Set once at MCP `initialize`, used for the connection lifetime      |
| Per-process forwarding shim       | Whatever the wrapping process derives from its environment         |
| Server CWD / env at startup       | Single global value, useless for multi-project hosts                |

Three approaches were considered:

1. **Per-call parameter only (status quo of ADR-0021).** Honest but
   under-defended: every client must opt in on every call. Acceptable for
   power-user tooling, hostile to default UX.

2. **MCP `initialize` handshake carries `scope`.** Clean for stdio MCPs
   (`claude_mcp` spawns one process per workspace). Awkward for HTTP because
   one HTTP server hosts many concurrent client connections; the connection
   identity must be carried on every request anyway, which is functionally
   the same as a header.

3. **HTTP header `X-Aver-Scope` plus an optional per-process forwarding shim
   (`aver-scope-shim`) that injects it.** The header is the contract; the
   shim is a reference implementation for clients that cannot send headers
   themselves. This composes with ADR-0021's per-call parameter: the
   parameter, if present, wins.

Option 3 is the decision.

## Decision

Specify a layered scope-resolution chain that every aver-server request
passes through, with documented precedence and a reference shim that makes
"scope follows the working directory" automatic for harnesses that cannot
send headers.

### Resolution precedence (per request)

```text
1. Tool parameter `scope` (ADR-0021)        — explicit per-call override
2. HTTP header `X-Aver-Scope`                — per-connection identity
3. HTTP header `X-Aver-Scope-Default`        — fallback if (1) and (2) absent
4. Server config `AVER_DEFAULT_SCOPE` env    — host-wide fallback
5. Hardcoded default `global`                — preserves ADR-0021 baseline
```

The first non-empty value wins. This intentionally lets a power-user override
a misconfigured shim by passing `scope` directly on the call, and lets a
cautious client send a default header without binding every request.

### Header semantics

`X-Aver-Scope` is a single normative scope string conforming to the path
convention in ADR-0021. The server validates against the same charset used
by the schema triggers (`[A-Za-z0-9_/-]`) and rejects requests with malformed
headers via HTTP 400 — failing fast rather than silently writing under
`'global'`. `X-Aver-Scope-Default` follows the same validation but is only
consulted when the request has neither a tool parameter nor `X-Aver-Scope`.

The header is recorded on every write into `events.source` (or analogous
provenance fields) so that scope decisions are auditable from the JSONL log
alone, independent of the SQLite materialization.

### `aver-scope-shim`

A new binary in `crates/aver-scope-shim/` (sibling of `aver-cli`,
`aver-server`) that:

- Binds to `127.0.0.1:0` (an ephemeral local port).
- On startup, resolves a scope from its working directory:

  ```text
  if `git rev-parse --show-toplevel` succeeds:
      slug = first 12 hex chars of sha256(`git config remote.origin.url`)
             if origin exists,
             else first 12 hex chars of sha256(absolute toplevel path)
      scope = "proj/{slug}"
  else:
      scope = AVER_DEFAULT_SCOPE env var, or "global"
  ```

  The fallback hashes the *absolute toplevel path*, not the basename.
  This closes the council-flagged collision risk (review 2026-05-10) where
  two unrelated repositories with the same directory name (e.g. `~/a/mcp`
  and `~/b/mcp`) would otherwise share a scope. The cost is that moving a
  clone to a new parent directory changes its scope; that is acceptable
  because the user can always pin the slug with `--scope` or by setting
  an origin URL.

- Forwards every request to `AVER_UPSTREAM_URL` (default
  `http://127.0.0.1:3317/mcp`) with `X-Aver-Scope` injected.
- Prints its bound URL on stdout for the harness to pick up.

The shim is the single supported way for harnesses that lack header support
to participate in scoping. It is intentionally a separate binary rather than
a server flag because its lifecycle is per-workspace, not per-host.

### Integration with Claude Code (informative)

Each project's `.claude/settings.json` (or `~/.claude.json` per-project entry)
points its `aver` MCP at the shim instead of the upstream server. Today that
config (`~/.claude.json:mcpServers.aver`) reads:

```json
{ "type": "http", "url": "http://127.0.0.1:3317/mcp" }
```

After adoption, the per-project entry becomes equivalent to:

```json
{
  "type": "http",
  "command": ["aver-scope-shim", "--upstream", "http://127.0.0.1:3317/mcp"]
}
```

— or whatever the harness's mechanism is for a child process that produces an
HTTP URL on stdout. This ADR does not mandate the exact form; it only specifies
the contract the shim implements.

### Read-path default flip

ADR-0021 left this open. With Layer 2 in place, the read-path default for
`recall` and `expand` flips from "`'global'` + `scope_walk='any'`" (preserve
today's behavior) to "resolved-scope + `scope_walk='ancestors'`" (current
project plus inherited globals). The flip is gated on the resolved scope
being something other than `'global'`: a tool that resolves to `'global'`
gets `scope_walk='any'` so consolidation and audit paths still see
everything.

This is the only behavior-change in Layer 2. It is intentional: without it,
Layer 1 is a schema change that nobody benefits from until clients start
manually passing parameters.

### Out of scope for this ADR

- The contents of the `agent_id` field. Per-harness identity is a separate
  concern (an Opus-driven Claude Code session and a Hermes-driven Pi agent
  both write `agent_id='mcp'` today). Tracked separately.
- Authentication. The OAuth surface from ADR-0015 is orthogonal; scope is
  about *which slice of memory*, not *who is allowed to read it*.
- Cross-host / multi-user scope. ADR-0006 still binds.
- Renaming or moving claims between scopes after the fact. A simple `UPDATE`
  works; tooling for it is follow-on.

## Consequences

- (+) Default behavior for a project-aware harness becomes "this session's
  reads and writes scope to this project, plus inherited globals" with no
  per-call parameter discipline required from the LLM.
- (+) Power users keep the per-call escape hatch — the parameter wins over
  the header, by design. Audit and consolidation tools keep their
  cross-cutting view via `scope_walk='any'`.
- (+) The shim is a tiny self-contained binary that any HTTP-MCP harness can
  use, not just Claude Code. Hermes, Pi, and Codex harnesses get the same
  isolation the day they switch their MCP URL to the shim.
- (+) Resolution is fully observable: `X-Aver-Scope` recorded in
  `events.source`, scope value persisted on every row. ADR-0019's
  JSONL-as-source-of-truth invariant survives — the log can be replayed and
  every row's scope reconstructed.
- (+) Backwards compatibility is bounded and explicit. A pre-Layer-2 client
  hits the hardcoded `'global'` default and behaves exactly as today, modulo
  the read-default flip on `'global'`-resolved sessions (which is itself a
  no-op for them).
- (−) Two ways to set scope (parameter, header) means two ways to be wrong.
  Mitigation: the precedence chain is documented; `aver-server` logs the
  resolved scope at debug level so divergence is diagnosable.
- (−) The shim is one more process to launch per workspace. For Claude Code
  with several projects open, that is N shims. Memory and CPU are
  negligible, but there is a startup-time cost on first MCP call. Mitigation:
  the shim is a single static Rust binary; cold start is sub-100ms.
- (−) The shim derives scope from `git config remote.origin.url`. Repositories
  without an origin (fresh clones, local-only experiments) fall back to the
  worktree basename, which is brittle if two projects share the same dir
  name in different parents. Mitigation: `AVER_DEFAULT_SCOPE` override; explicit
  `--scope` flag on the shim.
- (−) The read-default flip is observable to clients who happened to depend
  on cross-project bleed. ADR-0021 documented this as a forthcoming change;
  this ADR is where the change actually ships. Anyone whose workflow relied
  on accidental cross-project recall will need to set
  `scope_walk='any'` explicitly or move the relevant claims to `'global'`.
- (−) `X-Aver-Scope` validation rejects malformed headers with HTTP 400.
  A misconfigured shim that derives a path with disallowed characters
  produces hard failures, not silent fallbacks. This is by design — silent
  fallback to `'global'` would re-introduce exactly the cross-repo pollution
  this ADR exists to fix — but it does mean operator error is loud.
