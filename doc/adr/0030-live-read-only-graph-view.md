# 30. Live read-only graph view

Date: 2026-11-15

## Status

Accepted

Edited 2026-11-15 to remove deployment details.

## Context

ADR-0029 added `contrib/graph`: `export.py` writes a read-only snapshot of a store to a private JSON file, and `graph.html` renders that file offline. The owner had to export again for every look. They asked to open a URL and see the current memory of a running agent: same page, same model, read from the live store on reload. It must stay strictly read-only (no promote, reject, retire, migration or other write), off by default, behind the same auth `aver-server` already enforces, and never reachable from the internet.

The motivating consumer is an agent on an owner-operated server that runs Aver only as `aver-server --stdio` child processes (ADR-0028) against a store it writes concurrently. That deployment has no HTTP listener.

## Decision

`aver-server --graph-view` is a separate, opt-in serving mode.

- **Off unless the flag is given.** Plain HTTP mode and `--stdio` serve no `/graph` or `/api/graph` (404). `--stdio` wins if both flags are given.
- **Routes.** The existing OAuth routes (ADR-0020), `/healthz`, `GET /graph` and `GET /api/graph`. No `/mcp`, and the mode never opens `AverTools` or any writer.
- **One model.** `/api/graph` returns the `aver-graph-v1` JSON of `export.py`, ported to Rust with the same legacy-column fallbacks. A committed snapshot built from invented data (`contrib/graph/testdata`) pins both implementations to the same output.
- **Read-only by construction.** Each request opens the store with `SQLITE_OPEN_READ_ONLY` (never creates the database file) and reads inside one transaction, so a reload shows the latest committed state and never a half-applied write. Read errors answer 503 with the detail on stderr only. Responses are `no-store`.
- **Auth.** `/api/graph` requires a bearer token validated like any other `aver-server` token, with scope `claims:read` (there is no narrower events scope; event payloads are covered by it). Missing or invalid token: 401; wrong scope: 403. The page runs the existing public-client PKCE flow itself and keeps the token in `sessionStorage`.
- **Page.** `/graph` serves the same `graph.html` compiled into the binary. The served copy allows `connect-src 'self'` and is sent with a strict CSP header (`default-src 'none'`, `frame-ancestors 'none'`); the file opened from disk keeps `connect-src 'none'` and the manual export path still works.
- **Fail closed at startup.** The store directory and `db.sqlite` must exist. The auth database must resolve outside the store directory, so the default `<memory_dir>/auth.db` is refused in this mode and deployments set `AVER_AUTH_DB_PATH`. On Linux the process must run as root or as the owner of `db.sqlite`.

Why the owner check: a read-only SQLite reader of a WAL database creates `-wal`/`-shm` when no writer currently holds them, and a consumer's stdio writers come and go. Running as root, SQLite gives those files to the database owner; running as any other uid they would be foreign-owned and could block the real writer. Readers take read marks in `-shm` (normal SQLite locking) and never checkpoint. So the view never writes memory data (`db.sqlite` bytes, the log), but it may create and update those two coordination files, which stay after it stops.

Access to such a store: the view runs as a separate host process on that server, bound to `127.0.0.1`, as the store owner. The owner reaches it with `ssh -L <port>:127.0.0.1:<port> <host>` and `http://localhost:<port>/graph` (`AVER_BASE_URL` must equal that origin for the consent check). No VPN listener, public hostname or tunnel route. The consumer's config and its stdio binary are not changed by this view.

Rejected: a flag on normal HTTP mode (would put the MCP writer in the same process); a paste-a-token box (needs a second way to mint tokens); serving from inside the consumer's container (needs a consumer config change and recreate for a read-only view).

## Consequences

- (+) The owner sees live memory with one SSH command; turning it off is stopping one process.
- (+) No new dependency, service or data model; the offline export keeps working.
- (+) The view cannot change memory: no writer is opened, and the database connection is read-only. Only SQLite's `-wal`/`-shm` coordination files may be created or touched.
- (−) A second way to read the store; parity with `export.py` must be kept by the shared golden test.
- (−) The view process needs the store owner's uid (or root) and its own auth database outside the store directory.
