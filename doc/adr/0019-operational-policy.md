# 19. Operational policy: checkpoint, vacuum, backup, replay

Date: 2026-05-10

## Status

Accepted

## Context

Aver is local-first (ADR-0006): one user, one machine, no network surface. The
system of record is the JSONL log (ADR-0005) and SQLite is a materialized view
that can be regenerated from it. `Store::open` in
`crates/aver-core/src/lib.rs:185-209` enables WAL, turns on foreign keys, and
runs the embedded `MIGRATIONS` slice (`crates/aver-core/src/lib.rs:41-78`)
unconditionally. Each migration uses `IF NOT EXISTS`, so re-application is a
no-op.

That covers the open path. Nothing covers steady-state operation. The current
on-disk layout under `.aver/` is:

```
.aver/
├── agents/<agent_id>/log.jsonl   # ADR-0014 per-agent capture
├── db.sqlite                     # materialized view; WAL adds db.sqlite-wal, db.sqlite-shm
├── events.jsonl                  # episodic events (1.5 MB today)
├── log.jsonl                     # the system of record (3.0 MB / 7,954 lines today)
└── observations.jsonl            # ADR-0016 observation projections
```

There is no policy for the things SQLite-backed stores must answer eventually:

| Concern              | Current state                                                         | Risk if ignored                                                         |
|----------------------|-----------------------------------------------------------------------|-------------------------------------------------------------------------|
| WAL checkpoint       | Default `wal_autocheckpoint = 1000` pages (~4 MB) implicit            | WAL can grow beyond useful bound under bursty writes; readers see stale snapshots |
| `VACUUM`             | Never run                                                             | Free-list accumulation, db file bloat after consolidation deletes/updates       |
| Backup               | Unspecified; relies on `tar` of the directory while a session is live | Single corrupted page loses everything between consolidations                   |
| Replay / DR          | ADR-0005 says "rebuildable from log" but no command implements it     | "Source of truth" is aspirational until proven                                  |
| Log rotation         | Unbounded `log.jsonl`                                                 | At ~1 KB/line, a long-running project crosses 1 GB inside a year                |
| Schema version       | `MIGRATIONS` runs unconditionally; no `schema_version` table          | Stale db cannot self-diagnose missing migrations; downgrade is undefined        |

ADRs 0005 and 0006 set the foundation. This ADR specifies the operational
steady-state on top of it. It is doc-only — no code changes ship with this
commit. The intent is to lock down the contract before any of these features
are implemented, so the eventual implementation has a target to hit.

### Workload assumptions

These policies are tuned for the workload that ADR-0006 commits to: one user,
one machine, write rates dominated by interactive sessions plus periodic
consolidation. Concretely:

- **Write rate.** Burst rate during active extraction is on the order of
  10-100 rows/sec into `claims` and `episodic_events`; sustained rate over a
  full session is closer to 1-10 rows/sec. Consolidation runs (ADR-0005) write
  a few hundred to a few thousand rows in one go.
- **Read rate.** Recall queries are sub-second and run a handful of statements
  each. The `Store` connection is held by a single process; concurrent readers
  are not a goal of v1 (ADR-0006 mentions WAL "for concurrent reads during
  writes" — that means the consolidator can read while a session writes, not a
  multi-process cluster).
- **Db size.** Today: 120 KB. Projected for a year of single-developer use:
  10-100 MB. Hard ceiling per ADR-0006: ~10⁶ active edges before swap-out is
  required. None of these are pressuring SQLite's limits.
- **Log size.** Today: 3 MB / 7,954 lines on `log.jsonl` after a few weeks of
  use. Linear extrapolation puts an active project under 100 MB/year. Bursty
  workloads (large code-base ingestion) can spike higher and motivate §5.
- **Failure model.** The dominant failure is process kill (Ctrl-C, OS reboot,
  crash). Disk corruption is rare but not zero on btrfs; full machine loss is
  out of scope for Aver and handled by the user's existing backup strategy.

## Decision

Six concrete policies follow. Each names a mechanism, a cadence, and a default
value. Where the right answer is "do nothing yet," the trigger condition for
revisiting is stated explicitly.

### 1. WAL checkpoint

**Decision.** Rely on SQLite's `wal_autocheckpoint`, but raise the threshold
from the 1,000-page default to **4,000 pages (~16 MB)**, and add an explicit
checkpoint on `Store::close` (today implicit via `Drop`; we will add an explicit
`close()` that runs `PRAGMA wal_checkpoint(TRUNCATE);` before dropping the
connection).

Rationale:

- Aver's write pattern is a stream of small inserts (one row per claim, one row
  per episodic event) plus occasional larger consolidation batches. The default
  4 MB threshold checkpoints constantly under modest load and adds latency to
  the very next writer.
- 16 MB is still small relative to disk caches and recovery time but cuts
  checkpoint frequency by ~4x for typical sessions.
- An explicit `wal_checkpoint(TRUNCATE)` on close guarantees that a clean
  shutdown leaves no WAL behind. If the process is killed, the next `open`
  recovers via the existing WAL mechanism — no special handling needed.

No background-thread checkpoint loop. The autocheckpoint already runs on the
writing thread when the threshold is crossed; adding a second checkpointer
introduces lock contention without buying anything.

```rust
// crates/aver-core/src/lib.rs, after WAL is enabled:
conn.pragma_update(None, "wal_autocheckpoint", 4_000)?;
// ... and on close():
conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
```

**Revisit if:** profiling shows checkpoint stalls > 50 ms on the agent hot
path, or `db.sqlite-wal` is observed > 64 MB at rest.

**Alternatives considered.**

- *PASSIVE checkpoint on a timer.* Rejected: SQLite's PASSIVE checkpoint is
  best-effort and skips frames held by readers. On a single-writer workload it
  rarely accomplishes more than autocheckpoint already does, and the timer
  thread adds a synchronization surface.
- *FULL or RESTART checkpoint per write.* Rejected: defeats the point of WAL.
- *Disable WAL entirely (rollback journal).* Rejected: ADR-0006 explicitly
  picks WAL for "concurrent reads during writes," which is needed for the
  consolidator-while-session pattern.

### 2. `VACUUM`

**Decision.** Manual only. Add an `aver vacuum` CLI subcommand. Do **not** run
`VACUUM` automatically on open or on a schedule.

Rationale:

- `VACUUM` requires an exclusive lock and rewrites the entire database file.
  On a healthy local Aver db that is small (today: 120 KB) the cost is
  negligible. But under shared mode (ADR-0011, future) or after a large
  consolidation it could block all readers for seconds.
- Auto-vacuum-on-open is a foot-gun: a user re-opening their tools after a
  crash does not want a multi-second stall.
- Free-list growth in this workload is dominated by consolidation runs that
  mark old claims `SUPERSEDED` (ADR-0005) — those rows are not deleted, so the
  free list grows mostly from index churn, which is bounded.

Contract for `aver vacuum`:

| Property      | Value                                                                                          |
|---------------|------------------------------------------------------------------------------------------------|
| Command       | `aver vacuum [--analyze] [--into <path>]`                                                      |
| Default       | In-place `VACUUM;` followed by `PRAGMA optimize;`                                              |
| `--analyze`   | Adds `ANALYZE;` after vacuum to refresh planner stats                                          |
| `--into PATH` | `VACUUM INTO 'PATH'` — produces a defragmented copy without taking an exclusive lock on origin |
| Locking       | Refuses to run if any other Aver process holds a write lock; advisory lockfile `.aver/.lock`   |
| Output        | Reports pre/post page count and free-list size                                                 |

`VACUUM INTO` is the recommended path for any unattended/scripted use because
it does not block readers. The in-place form is for the human operator who
knows nothing else is running.

**Revisit if:** `PRAGMA freelist_count` ever exceeds 25% of `PRAGMA page_count`
in the field, in which case schedule `VACUUM INTO` after each consolidation.

**Why not `PRAGMA auto_vacuum = INCREMENTAL`?** It commits Aver to a setting
that can only be changed by VACUUMing the entire database, and it costs an
extra page-pointer table on every db. The free-list growth pattern doesn't
warrant the lock-in. If incremental vacuum becomes attractive later, it can
be enabled in a future migration via a one-time full `VACUUM` that flips the
mode.

### 3. Backup

**Decision.** Treat `log.jsonl` (and its rotated siblings, see §5) as the
**primary backup**. `db.sqlite` is treated as ephemeral cache, regenerable via
replay (§4). Do not ship a SQLite-snapshot backup tool in v1.

Rationale:

- ADR-0005 already commits to the log as the system of record. Adding a
  parallel SQLite-snapshot backup creates two backup paths with two recovery
  procedures — and the SQLite snapshot is strictly weaker (it captures derived
  state plus any consolidation-time mistakes baked in).
- The user's environment uses btrfs subvolumes on a single machine. Disk-level
  backup (snapshot, rsync, restic) is out of scope for Aver and already
  available to the user; Aver should not reinvent it.
- The log is append-only and append-friendly to incremental backup tools.
  Snapshotting a live SQLite db without the online-backup API is unsafe; the
  online-backup API exists in `rusqlite::backup` but adds a second moving part
  that has to be tested.

Concretely:

| Mechanism                                  | Status                                                                                       |
|--------------------------------------------|----------------------------------------------------------------------------------------------|
| `log.jsonl` + rotated logs (§5)            | **Primary backup.** Plaintext, append-only, replayable.                                      |
| User-level disk backup (restic, btrfs)     | Recommended out-of-band. Aver documents how to include `.aver/` and exclude `db.sqlite-wal`. |
| `aver backup --into <path>` (online copy)  | Deferred. Not built in v1.                                                                   |
| `.aver/backups/db.YYYY-MM-DDTHH.sqlite`    | Not used.                                                                                    |

Recovery procedure documented in v1: `rm -rf .aver/db.sqlite .aver/db.sqlite-wal
.aver/db.sqlite-shm && aver replay`. That is the contract.

**Revisit if:** replay time grows past ~30 seconds on a representative log, at
which point cold-start latency starts to hurt, and a checkpointed SQLite
snapshot becomes worth the second code path. Also revisit if shared mode
(ADR-0011) is ever turned on, because then the log is no longer single-writer.

### 4. Replay / disaster recovery

**Decision.** Add an `aver replay` subcommand. Contract:

| Property        | Value                                                                                                                |
|-----------------|----------------------------------------------------------------------------------------------------------------------|
| Inputs          | `.aver/log.jsonl` plus all rotated `.aver/log.{N}.jsonl.gz` (in numeric order, oldest first), then `events.jsonl`, then `observations.jsonl`, then per-agent `agents/<id>/log.jsonl` |
| Output          | A fully populated `db.sqlite` containing claims, episodic events, observations, candidate claims, contradictions, ontology seed |
| Determinism     | Pure function of input log content; running twice produces byte-identical `db.sqlite` after `VACUUM`                 |
| Idempotency     | Replay over an existing `db.sqlite` is allowed only with `--force`; default refuses to start if `claims` is non-empty |
| Identifier policy | The `id` from each log record is preserved verbatim. Conflicting ids fail loudly (`E_REPLAY_DUPLICATE_ID`) — they indicate log corruption, not a valid state |
| Privacy filter  | The privacy filter (ADR-0009) **does not run** during replay. The log is presumed already filtered at write time. A replay-time re-filter would silently drop rows that are present in the source of truth |
| Schema version  | Replay applies all `MIGRATIONS` first, then loads. A replay against a newer Aver binary is supported; against an older one it errors with `E_REPLAY_SCHEMA_TOO_NEW` |
| Progress        | Streamed line counter, no progress bar dependency                                                                    |
| Failure mode    | Atomic: on error, leaves `db.sqlite.partial` and exits non-zero; never overwrites a working `db.sqlite` mid-flight   |

What replay does **not** do:

- It does not re-run consolidation. If the log records `SUPERSEDED` transitions,
  those are loaded as written. Re-deriving the graph from raw episodic events
  is a separate operation (`aver consolidate`, ADR-0005) and is not implied by
  replay.
- It does not regenerate vector embeddings. Embeddings live in
  `vector_chunks` (migration `0002_vector_chunks`) and are re-derived on
  demand from the source text already loaded by replay. A separate `aver
  reembed` command (out of scope for this ADR) handles that.

`events.jsonl`, `observations.jsonl`, and per-agent `agents/<id>/log.jsonl` are
all replayable into their corresponding tables (`episodic_events`,
`observations`, plus `claims` for any agent log records that represent claim
writes). The contract above lists them as inputs in deterministic order: global
events first, then per-agent in lexicographic agent-id order. This matters
because some derived rows reference others by id (e.g., a contradiction
references two claim ids); load order must produce a valid foreign-key graph.

**Pseudocode for replay (not implemented in this commit):**

```text
fn replay(memory_dir):
    open db with all MIGRATIONS applied, schema_version asserted
    for log in [rotated logs oldest→newest, log.jsonl]:
        for line in log:
            apply(record)   // INSERT into target table by record.kind
    for log in [events.jsonl, observations.jsonl, agents/*/log.jsonl]:
        for line in log:
            apply(record)
    PRAGMA optimize
    PRAGMA wal_checkpoint(TRUNCATE)
```

Apply is a `match` on `record.kind` ∈ {`claim`, `episodic_event`, `observation`,
`candidate_claim`, `contradiction`, `privacy_rejection`, ...}, with one INSERT
statement per kind. The exhaustive enumeration is intentional: a record with
an unknown `kind` is an error, not a skip — silent skip is how subtle data
loss happens.

### 5. Log rotation

**Decision.** Rotate `log.jsonl` when it exceeds **64 MB** OR **500,000 lines**,
whichever comes first. Rotated files live alongside the active log as
`.aver/log.{N}.jsonl.gz`, where `N` starts at 1 and increases monotonically.
Compress with gzip at level 6 (default). Never delete. Retention is "forever"
because the log is the source of truth (§3).

| Trigger                            | Action                                                                             |
|------------------------------------|------------------------------------------------------------------------------------|
| `log.jsonl` ≥ 64 MB                | Rotate                                                                             |
| `log.jsonl` ≥ 500,000 lines        | Rotate                                                                             |
| Process startup                    | Check size/lines; rotate if over threshold (handles the case of a crashed rotation) |
| Aver process boundary              | Rotation runs only at session boundaries — never mid-session — to avoid coordinating writers |

Rotation algorithm:

1. Acquire `.aver/.lock` (the same advisory lock used by `vacuum`).
2. Determine next `N` by scanning `log.{*}.jsonl.gz` and taking max+1.
3. `mv log.jsonl log.{N}.jsonl` (atomic rename on the same filesystem).
4. `gzip log.{N}.jsonl` → `log.{N}.jsonl.gz`.
5. Touch a fresh empty `log.jsonl`.
6. Release lock.

If the gzip step fails or the process is killed between steps 3 and 5, recovery
is: on next startup, if `log.{N}.jsonl` exists without `.gz`, finish gzipping
it; if `log.jsonl` is missing, recreate it empty.

Rotation **never** rotates `events.jsonl`, `observations.jsonl`, or per-agent
logs in v1. Those are bounded by session and agent count respectively, and
their growth profile does not justify the operational complexity yet.

**Implications for replay (§4):** replay must walk rotated logs in order
before the active log. The order is `log.1.jsonl.gz`, `log.2.jsonl.gz`, ...,
`log.N.jsonl.gz`, `log.jsonl`. Numeric, not lexicographic — `log.10.jsonl.gz`
sorts before `log.2.jsonl.gz` lexicographically and that is wrong.

**Revisit if:** typical project log exceeds 1 GB compressed total, in which
case archival to cold storage with a `aver log archive --before <date>`
command starts to make sense.

**Why size + line dual trigger?** Pure size triggers are fooled by
write-amplification differences (a project that logs many small claims rotates
later than one logging fewer large ones, even though the line count — and
therefore replay time — is comparable). Pure line triggers ignore the case of
a few unusually large records (ADR-0014 candidate-claim records can be
multi-KB). Whichever fires first is the right answer for both replay
performance and disk usage.

### 6. Schema version metadata

**Decision.** Adopt SQLite's built-in `PRAGMA user_version`. After running
`MIGRATIONS`, `Store::open` writes `PRAGMA user_version = N;` where `N` is the
length of the `MIGRATIONS` slice. On open, before running migrations, read
`user_version`:

| Read value `v`            | Action                                                                                       |
|---------------------------|----------------------------------------------------------------------------------------------|
| `v == 0`                  | Fresh db (or pre-policy db). Run all `MIGRATIONS`, set `user_version = MIGRATIONS.len()`     |
| `0 < v < MIGRATIONS.len()` | Run migrations `[v..]` only. Set `user_version = MIGRATIONS.len()`                          |
| `v == MIGRATIONS.len()`   | No migrations needed                                                                         |
| `v > MIGRATIONS.len()`    | Error `E_SCHEMA_TOO_NEW`. Refuse to open. The user is on an older binary than the db        |

Why `user_version` and not a `schema_version` table:

- `user_version` is a single 32-bit integer stored in the db header. Cheap to
  read, atomic to update, and survives `VACUUM`.
- A `schema_version` table would record per-migration metadata (name, applied
  at, hash). That is genuinely useful for forensics but adds a row write to
  every fresh db and a table the rest of the codebase has to know about.
  Today's `MIGRATIONS` is a static `&[(&str, &str)]` const; reproducibility is
  guaranteed at compile time, not at runtime metadata.
- If forensic detail is ever needed, it can be added later as a non-breaking
  layer on top of `user_version`. The reverse — removing a `schema_version`
  table — is a breaking change.

**Migration migration**: existing dbs in the wild today have `user_version = 0`.
The first release that ships this policy will treat `0` as "run all migrations,
then set version" — that is identical to today's behavior except for the
trailing `PRAGMA user_version` write. Backwards compatible.

```rust
// crates/aver-core/src/lib.rs, replacing the unconditional loop at line 198:
let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
let target = MIGRATIONS.len() as i64;
if current > target {
    return Err(Error::SchemaTooNew { found: current, supported: target });
}
for (_name, sql) in &MIGRATIONS[current as usize..] {
    conn.execute_batch(sql)?;
}
conn.pragma_update(None, "user_version", target)?;
```

**Revisit if:** migrations ever need to be non-idempotent (e.g., a data
backfill that cannot use `IF NOT EXISTS`), in which case per-migration
tracking via a `schema_version` table becomes mandatory.

## Consequences

- (+) WAL no longer grows unboundedly under bursty writes, and clean shutdown
  leaves no WAL file behind. Cold-start I/O is predictable.
- (+) `VACUUM` is available to operators but never surprises them. The
  in-place form has a documented locking contract; `VACUUM INTO` is the safe
  default for any automation.
- (+) Backup story is honest: the log is the backup, recovery is `replay`, and
  out-of-band disk backup is the user's existing tool. No second backup
  pathway to maintain.
- (+) `aver replay` makes ADR-0005's "rebuildable from log" claim falsifiable.
  Without this command, that claim is decorative.
- (+) Log rotation puts a ceiling on active-file size and gives replay a
  deterministic order to walk. Numeric (not lexicographic) ordering is called
  out explicitly because the bug-by-default is real.
- (+) `PRAGMA user_version` lets a stale db detect a too-new binary and refuse
  to open, instead of silently re-running idempotent `IF NOT EXISTS` DDL and
  hoping nothing has actually changed.
- (−) Six policies, six places to get something subtly wrong. The replay
  contract in particular is non-trivial — id preservation, foreign-key load
  order, and the privacy-filter-during-replay decision are all easy to
  regress. CI must include a "round-trip" test: open store, write N records,
  rebuild from log, byte-compare the dbs after `VACUUM`.
- (−) "Log is the backup" is correct only if the log is genuinely complete.
  Today, vector embeddings (table `vector_chunks`) are derived but not all
  derivation inputs are logged — re-embedding has to be re-run after replay,
  and that is a separate command this ADR does not specify. There is a
  silent-loss risk if a future feature writes derived data that has no log
  source.
- (−) Rotation runs only at session boundaries. Long-running daemonized
  agents (a future possibility) will hold a single `log.jsonl` open
  indefinitely and never trigger rotation. Solving that needs a SIGHUP
  handler or an in-process scheduler — out of scope here, flagged as a known
  limitation.
- (−) `user_version` carries no metadata about *which* migrations were
  applied. If a migration is ever renamed or reordered, the version number
  alone cannot detect the inconsistency. This is acceptable while migrations
  are append-only and code-reviewed; it stops being acceptable the first
  time someone needs to backfill or reorder.
- (−) None of these policies are implemented yet. This ADR is a spec, not a
  shipped feature. Until the matching code lands, the operational risks
  (unbounded WAL, no replay command, no rotation) remain real.
- (−) The privacy-filter-during-replay decision (§4) is load-bearing and
  arguably wrong if a detector is added that would have rejected a record
  written under an older filter. ADR-0009 §"filter placement" already says
  the log is treated as immutable once written; this ADR ratifies that for
  replay specifically. If that ever changes, the replay contract has to
  change with it.
