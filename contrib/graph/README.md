# Local memory graph

This is a sanitized replacement for the graph prototype: a read-only exporter
and an offline page with no external JavaScript, dependencies or graph service.
It includes no memory data and knows no domain vocabulary: tables, columns and
groups come from whatever the store holds. Literal values stay in tables and the
detail panel rather than becoming graph nodes; source events and contradictions
are available for audit.

Export an explicit local database to a **new file outside the repository**:

```sh
python3 contrib/graph/export.py /path/to/aver/db.sqlite "$HOME/aver-private-graph.json"
```

The exporter uses a read-only SQLite snapshot, creates the output with mode
0600, refuses to overwrite an existing file, and prints only structural counts.
The JSON can include private memory and event payloads. Keep it private; never
commit it, a database, credentials, or screenshots of real memory.

Open `contrib/graph/graph.html` in a local browser, then choose the exported JSON
with **Local export**. No server or upload is required. The page creates text
nodes for data and blocks network access through CSP.

- **Entities** (default): pick a group, either subjects sharing an `is` type or
  subjects sharing an id prefix such as `task:`. One row per subject; the
  columns are the predicates that occur in the group, most frequent first, plus
  incoming relations (`← supersedes`). Sort by any column (`time` is the
  subject's earliest claim) and filter by any column value. A table shows at
  most 1000 rows and a column filter at most 200 values.
- **Notes**: claims whose object is free text by ADR-0029's prose rule (the
  page ports `ObjectKind::is_prose` exactly; `testdata/prose_cases.json` pins
  both) carry an "unstructured" badge and stay out of the table and graph.
- **Graph**: clicking a row or node shows its neighbourhood (1 or 2 hops):
  incoming relations to the left, outgoing to the right, labels pointing away
  from the edges and the node's own values under it. **Whole store** lays every
  entity out in deterministic layers by edge direction, labels on hover or
  focus. Circles are entities; diamonds are source events, and an `event:<id>`
  object is the same node as that event.
- **Status / Created**: "All history" adds superseded and retired claims,
  struck through with their reason (`retired:` marker or contradiction). The
  date range filters every tab by claim creation time.

Aver's ontology declares each predicate's cardinality but not its object kind,
so the page infers the kind from the stored facts: a predicate is literal when
most of its facts are stored as literal (on a tie each fact keeps its own kind).
A fact stored as an entity under a literal predicate is shown as a value with a
"stored as entity" hint; nothing is rewritten.

Historical projections without object typing retain their legacy graph meaning.
Run the backed-up [structured migration](../../doc/structured-memory.md) to
reclassify prose; the exporter does not infer or modify missing historical data.
This is an inspection view, not an editor or a migration tool. It focuses on
claims and their source events; hyperedge visualization is not included.

## Live view

`aver-server --graph-view` serves this page at `/graph` and the same
`aver-graph-v1` snapshot as `export.py` at `/api/graph`, read live from the
store on every page load. It is off unless the flag is given and replaces the
MCP server for that process: no `/mcp`, no writes, no migration.

```sh
AVER_MEMORY_DIR=/path/to/aver AVER_AUTH_DB_PATH=/elsewhere/graph-auth.db \
AVER_BASE_URL=http://localhost:3317 aver-server --graph-view
```

- **Auth.** `/api/graph` requires a bearer token with `claims:read` (401
  without, 403 without the scope). The page gets one itself through the
  server's OAuth PKCE flow: the first visit shows the consent screen, later
  visits reuse the consent. The token lives in `sessionStorage` only.
- **Private by placement.** It binds `127.0.0.1` by default. For a remote store,
  keep it on loopback there and use an SSH tunnel
  (`ssh -L 3317:127.0.0.1:3317 <host>`), then open
  `http://localhost:3317/graph`. Set `AVER_BASE_URL` to exactly that origin,
  because the consent form checks it.
- **Never writes memory data.** `db.sqlite` and the log stay untouched; only
  SQLite's own `-wal`/`-shm` coordination files may be created or updated
  (see the next point). It refuses to start if `AVER_AUTH_DB_PATH`
  resolves inside `AVER_MEMORY_DIR` (the default does) or if `db.sqlite` is
  missing. Each request opens `db.sqlite` read-only in one read transaction,
  so it shows committed state only while a writer is active.
- **Run as the store owner (or root).** In WAL mode SQLite itself creates
  `db.sqlite-shm`/`-wal` when no writer has them open, even for a read-only
  reader. Files made by another user would lock the writer out, so the server
  refuses to start on Linux unless its user owns `db.sqlite` or is root (SQLite
  then gives those files the database's owner).
- **Off switch.** Stop the process. Nothing else changes.

Offline synthetic exporter checks:

```sh
python3 -m unittest discover -s contrib/graph
```

`testdata/*.sql` are invented stores and `testdata/*.json` their expected
exports; the Python and the Rust (`--graph-view`) exporter tests both compare
against them. `synthetic` covers exporter edge cases, `projects` is a small
project/people/tasks memory and `decisions` a decision-shaped one; the same page
serves both. Exports carry `created_at` on claims and the ontology's
`predicates` (name, parent, cardinality); both are additive to `aver-graph-v1`.

Browser tests must use invented data and a private, headless browser.
The headless proof covers the tabs, creation-date range and **All history**:

```sh
NODE_PATH="$(npm root -g)" node contrib/graph/test_graph.js
```

CI runs it with Playwright 1.58.2 and headless Chromium. Set `CHROMIUM` to a
browser binary if Playwright did not download one.

The README screenshots in `doc/images/` come from `testdata/projects.json`.
Regenerate them with Playwright and Chromium installed outside this
repository (no repo dependency). The script runs a headless browser and opens
no window:

```sh
NODE_PATH="$(npm root -g)" node contrib/graph/screenshots.js
```

Set `CHROMIUM` to a browser binary if Playwright did not download one.
