# 25. Retire `aver-scope-shim`

Date: 2026-10-24

## Status

Proposed

## Context

ADR-0022 resolves a request's scope from, in order: the tool parameter
`scope`, the HTTP header `X-Aver-Scope`, the header `X-Aver-Scope-Default`,
`AVER_DEFAULT_SCOPE`, then `global`. The header is the contract. ADR-0022
also specified `aver-scope-shim` as a reference implementation "for clients
that cannot send headers themselves".

What the shim does (`crates/aver-scope-shim/src/{lib,main,proxy}.rs`, about
450 lines plus an axum, reqwest, tokio, tower, sha2 and futures-util
dependency set):

- Injects exactly one header, `X-Aver-Scope`, and strips any client-supplied
  `X-Aver-Scope` so a client cannot override it. `X-Aver-Scope-Default` is
  not touched. It also drops hop-by-hop, `host` and `content-length`
  headers, which any proxy must do.
- Derives the value once at startup: `--scope` flag, else
  `proj/<sha256(git remote.origin.url)[..12]>`, else
  `proj/<sha256(absolute toplevel)[..12]>`, else `AVER_DEFAULT_SCOPE`, else
  `global`.
- Binds an ephemeral localhost port and prints the URL on stdout.

The design review (2026-10-17) calls this "a per-workspace proxy process for
a header that an MCP client config line can set". Roadmap Phase 2 says to
fold the shim into client configuration if the client can set the header,
and otherwise to record why in an ADR.

Evidence on client capability:

- Claude Code. Remote HTTP servers take a static `headers` object in
  `.mcp.json` and `claude mcp add --transport http <name> <url> --header
  "Name: value"` (`-H`). `url` and `headers` support `${VAR}` and
  `${VAR:-default}` expansion. A `headersHelper` command can emit a JSON
  object of headers at connect time, which overrides static headers of the
  same name. Project scope (`.mcp.json` at the repo root) is per workspace.
  Source: https://code.claude.com/docs/en/mcp (docs.claude.com redirects
  there).
- Codex CLI. `config.toml` streamable-HTTP servers accept `http_headers`
  (static map) and `env_http_headers` (header name to env var name).
  Source: https://developers.openai.com/codex/config-reference
- OpenCode. A `remote` MCP entry in `opencode.json` accepts `headers`
  (record of strings). Source: https://opencode.ai/docs/mcp-servers/

ADR-0022's own Claude Code example (`"command": ["aver-scope-shim", ...]` on
an `http` server) is not a form Claude Code supports; the shim as described
there could only be used by launching it by hand and pointing `url` at the
printed port, which also changes on every launch.

The server does not treat the header as a credential. ADR-0022 states that
scope is "which slice of memory", not "who may read it"; authorisation is the
OAuth path of ADR-0015. Stripping a client-supplied header therefore protects
nothing the server relies on, and a client that can set the header directly
gains nothing it could not do with the `scope` tool parameter, which already
wins over the header.

The one thing the shim does that a static header does not is automatic
derivation from the git working directory. That is covered below.

## Decision

Retire `aver-scope-shim`. Every client we target can send a per-server
header, so the proxy process is not needed. Do not delete the crate in this
change; this ADR marks it deprecated and lists the removal steps.

### Replacement client configuration

The scope is a literal chosen once per project, for example `proj/aver`. It
must match `[A-Za-z0-9_/-]`, no empty segments, at most 256 bytes
(`scope_resolution.rs`). Replace `proj/aver` and the URL as needed.

Claude Code, project `.mcp.json` (committed or local):

```json
{
  "mcpServers": {
    "aver": {
      "type": "http",
      "url": "http://127.0.0.1:3317/mcp",
      "headers": { "X-Aver-Scope": "proj/aver" }
    }
  }
}
```

or the CLI equivalent:

```bash
claude mcp add --transport http --scope project aver http://127.0.0.1:3317/mcp \
  --header "X-Aver-Scope: proj/aver"
```

Codex CLI, `config.toml`:

```toml
[mcp_servers.aver]
url = "http://127.0.0.1:3317/mcp"
http_headers = { "X-Aver-Scope" = "proj/aver" }
```

Use `env_http_headers = { "X-Aver-Scope" = "AVER_SCOPE" }` to read it from the
environment (for example from a direnv `.envrc`).

OpenCode, `opencode.json`:

```json
{
  "mcp": {
    "aver": {
      "type": "remote",
      "url": "http://127.0.0.1:3317/mcp",
      "headers": { "X-Aver-Scope": "proj/aver" }
    }
  }
}
```

Any other MCP client that can send a custom header per HTTP server uses the
same single header. A client that cannot send headers can rely on
`AVER_DEFAULT_SCOPE` on the server or pass `scope` per call.

### Automatic derivation

The shim's git-derived slug is the only lost behaviour. Two options, neither
requires new code in this repository:

1. Use the literal. A human-readable `proj/<name>` is easier to audit than
   `proj/<12 hex chars>`, and one line per repository is a one-time cost.
2. For Claude Code only, `headersHelper` can reproduce the derivation. The
   script must print `{"X-Aver-Scope": "proj/<slug>"}`. The shim's slug for
   a repo with an origin is:

   ```bash
   printf '%s' "$(git config --get remote.origin.url)" | sha256sum | cut -c1-12
   ```

   Whether `headersHelper` runs with the project directory as its working
   directory is not stated in the docs reviewed here. Verify before relying
   on it.

If neither is acceptable we can add a small `aver scope` CLI subcommand that
prints the slug. That is deliberately not done here.

### Deprecation in this change

- `crates/aver-scope-shim/Cargo.toml` description is prefixed `DEPRECATED
  (ADR-0025)`.
- `README.md` repository tree marks the crate deprecated.

### Removal steps for the follow-up PR

1. Delete `crates/aver-scope-shim/` and its entry in the workspace
   `Cargo.toml` members.
2. Remove it from the justfile, CI workflow and `just dist` / installer
   artifact lists, if present, and drop the dependencies that only it used
   (`axum` is shared with `aver-server`; check `reqwest` features, `tower`,
   `futures-util`, `sha2`) from `Cargo.lock` via `cargo update -w`.
3. Remove the README tree line and any run instructions.
4. Amend ADR-0022: mark the `aver-scope-shim` and "Integration with Claude
   Code" sections superseded by ADR-0025. Its precedence chain and header
   semantics stay in force.
5. Keep `X-Aver-Scope` validation and the HTTP 400 behaviour in
   `scope_resolution.rs` unchanged.

## Consequences

- (+) One fewer binary, process and port per workspace, and about 450 lines
  plus five dependencies leave the workspace once removed.
- (+) No startup race or ephemeral-port discovery; the client talks to the
  server directly, so MCP streaming and OAuth behave as for any other HTTP
  server.
- (+) Scope appears in a file the user can read and commit, not in a derived
  hash.
- (−) Scope is no longer automatic. A project without the header silently
  resolves to the server default (`AVER_DEFAULT_SCOPE`, else `global`), which
  is the cross-repo pollution ADR-0022 set out to avoid. Mitigation: set the
  header in the project `.mcp.json`; a follow-up could make the server warn
  when a `proj/...`-less session writes to `global`.
- (−) Scopes are no longer stable across moved clones by construction; they
  are stable because the user wrote them down, which is better, but existing
  shim-derived `proj/<hash>` rows keep their hashes. Pin the old hash in the
  header (`--scope` output in the shim's stderr line shows it) to keep
  continuity, or migrate rows with an `UPDATE`.
- (−) Each client uses its own config syntax, so the per-client snippets above
  need maintenance when those clients change. Claude Code, Codex CLI and
  OpenCode details were taken from their public documentation and have not
  been exercised against `aver-server` in this change.
- (−) The header can be set by whoever controls client config. This is no
  weaker than today, since the tool parameter already overrides it.

## References

- ADR-0015 (OAuth), ADR-0021 (scope dimension), ADR-0022 (connection-scope
  resolution)
- Design review 2026-10-17: `doc/design-review-2026-10-17.md`
- Roadmap Phase 2: `doc/roadmap.md`
- Claude Code MCP: https://code.claude.com/docs/en/mcp
- Codex config reference: https://developers.openai.com/codex/config-reference
- OpenCode MCP servers: https://opencode.ai/docs/mcp-servers/
