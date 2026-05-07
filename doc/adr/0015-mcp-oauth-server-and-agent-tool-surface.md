# 15. MCP OAuth server and agent tool surface

Date: 2026-05-07

## Status

Accepted

## Context

AML needs to be usable from Claude, Codex, and other tool-using agents without embedding memory logic directly in prompts. The Model Context Protocol (MCP) is the integration point. Karta already demonstrates a practical architecture: an HTTP server with OAuth-style authorization, a protected MCP endpoint, and tool adapters over the memory core.

AML differs from Karta in one important way: AML's durable substrate is structured claims, not free-form notes. That is an advantage for provenance, contradiction handling, graph traversal, and consolidation, but it means the MCP surface must avoid pretending that every natural-language note can safely become durable memory immediately.

ADR-0014 defines the safe write path: record episodic events, trigger extraction, stage candidate claims, validate them, then promote accepted candidates into durable claims. The MCP server must expose this architecture rather than bypass it.

## Decision

AML adds a `memory-server` crate that provides an MCP/OAuth integration layer over `memory-core`.

### Server shape

The server follows the Karta pattern:

- Axum HTTP server.
- OAuth Authorization Server Metadata at `/.well-known/oauth-authorization-server`.
- OAuth token exchange at `/oauth/token` using authorization-code + PKCE S256.
- Bearer-token validation for protected routes.
- A protected MCP route for tool calls.
- Configuration via `AML_*` environment variables.

Initial environment variables:

```
AML_HOST
AML_PORT
AML_BASE_URL
AML_MEMORY_DIR
AML_AUTH_DB_PATH
```

OAuth client registration and full browser authorization routes may be added incrementally. The first implementation establishes the local auth database, token hashing, PKCE verification, and token exchange primitives before adding a full provider-backed login flow.

### Initial MCP tools

The first MCP tools are intentionally small:

```
remember_claim(subject, predicate, object, source?, agent_id?, agent_kind?)
recall(query, top_k?)
```

These are the smallest useful tools for Claude/Codex integration and are directly backed by existing `Store` behavior.

### Event/candidate MCP tools

The next tool group should expose the ADR-0014 lifecycle:

```
record_event(session_id, kind, payload, source?, agent_id?, agent_kind?)
should_extract_memories(session_id, threshold?)
propose_claims(session_id)
list_candidate_claims(status?, session_id?)
promote_candidate_claim(candidate_id)
reject_candidate_claim(candidate_id, reason)
```

These tools let an agent accumulate episodic context and explicitly trigger consolidation without silently writing every message into durable memory.

### Relationship to ADR-0008

ADR-0008's five-tool surface remains the high-level memory surface for stable agent use. ADR-0015 refines the write side for MCP integrations by adding lower-level event/candidate tools. These lower-level tools are operational controls for the triggered memory pipeline, not a replacement for durable graph recall/expand/consolidate tools.

## Consequences

- (+) Claude/Codex can integrate with AML through standard MCP instead of custom prompt glue.
- (+) OAuth/PKCE enables protected HTTP access suitable for browser-based MCP clients.
- (+) The initial claim tools are simple, deterministic, and easy to test.
- (+) The event/candidate tools will expose AML's safer triggered-write architecture instead of encouraging every-turn durable writes.
- (+) Reusing Karta's server shape reduces integration risk while keeping AML's core memory model independent.
- (-) OAuth provider-backed login, dynamic registration, and refresh tokens add operational complexity.
- (-) MCP's `ServerHandler` requires thread-safe service state, so SQLite-backed `Store` must be wrapped carefully at the server boundary.
- (-) The first server slice is not a complete production OAuth server until registration, authorization, refresh, CORS, and MCP route mounting are all implemented.

## Implementation notes

The current server foundation includes:

1. `crates/memory-server` workspace crate.
2. `ServerConfig::from_env` for `AML_*` configuration.
3. `AmlTools` adapter over `memory-core` with `remember_claim` and `recall`.
4. `AuthDb` with hashed access tokens and authorization-code storage.
5. PKCE S256 challenge/verification helpers.
6. OAuth discovery metadata.
7. Authorization-code token exchange with single-use codes.
8. `AmlMcpService` using `rmcp` tool macros for `remember_claim` and `recall`.
9. Axum router with discovery route, token route, protected health route, and runnable `memory-server` binary.

Next implementation slices:

1. Add `/oauth/register` dynamic client registration.
2. Add `/oauth/authorize` for local/dev authorization-code creation and later external IdP callbacks.
3. Mount protected `/mcp` streamable HTTP service.
4. Add event/candidate MCP tools from ADR-0014.
5. Add integration documentation for Claude/Codex clients.
