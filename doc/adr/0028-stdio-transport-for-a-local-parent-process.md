# 28. stdio transport for a local parent process

Date: 2026-11-12

## Status

Accepted

## Context

`aver-server` served MCP over streamable HTTP only, behind the OAuth 2.1 consent flow of ADR-0020. That flow needs a browser on a loopback connection to the server (or a trusted reverse-proxy header). An agent host that runs headless inside a container has neither: it cannot click a consent screen, a server inside the container is not supervised, and a server outside it needs a listener other containers can reach.

Such hosts start MCP servers as child processes and talk to them over stdin/stdout.

## Decision

`aver-server --stdio` serves the same MCP tool set to the process that spawned it, over stdin/stdout, instead of binding a listener.

- **Opt-in flag.** Without `--stdio` nothing changes: same HTTP routes, same OAuth, same scope checks.
- **No listener, no OAuth.** The stdio path never builds the HTTP router or opens the auth database.
- **The parent holds every scope.** There is no OAuth grant to consult, and the parent already controls the process, its environment and its store directory, so a scope check would protect nothing. This is a deliberate grant (`AverMcpService::local_stdio`), not a skipped check: an HTTP session still needs the scope its token carries, and the grant must never be used for a network transport.
- **Scope resolution.** There are no `X-Aver-Scope` headers. The session scope is `AVER_DEFAULT_SCOPE`, else `global` (ADR-0022 precedence without the header steps), resolved once at startup; a malformed value stops the process. A per-call `scope` parameter still wins.
- **stdout is the protocol.** Only JSON-RPC frames are written to stdout. Every diagnostic goes to stderr.
- **Lifetime.** The process ends when the parent closes stdin. A bad store path or setting ends it with a non-zero status and a message on stderr before any frame is written.
- **Several children on one store** are supported the same way a CLI next to a running server is: SQLite WAL with a single writer, the log's advisory lock, and writer-minted ids (ADR-0026).
- Writes are attributed to agent id `mcp` unless the call names one, as for any session without an OAuth client.

## Consequences

- (+) Headless agent hosts can use Aver with one config entry and no network surface.
- (+) The HTTP/OAuth path is untouched.
- (−) Anything that can start the binary against a store directory has full access to that store. File permissions on the store directory are the access control in this mode.
- (−) One more transport to keep working; covered by tests that drive the real binary over pipes.
