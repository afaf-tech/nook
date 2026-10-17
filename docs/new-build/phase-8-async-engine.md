# Phase 8 — A UI thread that never blocks

**Arc C · closes `sql.stop`, `sql.profiler`, `sql.planVisualiser`, `monitor.locks`, `monitor.replication`, `monitor.alerts` · depends on phase 6**

## The problem, measured

`sql.run` is called from the UI's update loop and blocks it:

| Fact | Evidence |
|---|---|
| The run happens on the frame's own thread | `editor.rs:254/347` set `actions.command = Some(("sql.run", …))`, `app.rs` runs it inline |
| A slow query freezes the window | `docs/gaps.md` §7: "`sql.run` blocks the UI thread, which is why no frame claims a statement is running" |
| Stop exists and is painted disabled | `sql.stop`'s help says so (`cmd/sql.rs`); the button is honest and useless |
| Cancelling needs a driver call | `docs/gaps.md` §7 names `pg_cancel_backend` / `KILL QUERY` |
| Explain is tabular only | `sql.explain` returns rows; no plan graph |
| No profiler, no lock report, no replication status, no alerts | `monitor.*` gap list |

There is already a working pattern for the fix in the same codebase: `ai.rs::spawn_ai_request` moves the
`curl` round-trip to an OS thread, hands back an `mpsc::Receiver`, and the frame polls it
(`ai.rs:136–156`). This phase generalises that pattern instead of inventing a second one.

TablePlus streams results as they arrive and keeps loading in the background; Beekeeper never blocks on a
query. This is the difference between a tool you can leave open and one you close.

## Scope

| Command | What it does | Params |
|---|---|---|
| `sql.runAsync` | **new** — same as `sql.run`, returns a `job` id immediately; results land in the tab when they arrive | `{"sql":"…","job":true}` |
| `sql.jobStatus` | **new** — state, rows so far, elapsed, error | `{"job":3}` |
| `sql.cancel` | **new** — cancel by job id: driver-level backend cancel, then a flag the worker honours | `{"job":3}` |
| `sql.stop` | **closes the Navicat id** — becomes a real command over `sql.cancel` for the active tab | `{}` |
| `sql.explainVisual` | **new** — `sql.explain`'s plan as a graph: nodes, costs, edges, and the expensive-node highlight | `{"sql":"…","analyze":true}` |
| `sql.planVisualiser` | **closes the Navicat id** — the UI over the command above | — |
| `sql.profiler` | **closes the Navicat id** — run a statement N times, report min/median/max/mean and the plan per run | `{"sql":"…","runs":10}` |
| `monitor.locks` | **closes the Navicat id** — blocking locks: who blocks whom, the query text, how long | `{}` |
| `monitor.replication` | **closes the Navicat id** — replica lag and state (Postgres `pg_stat_replication`, MySQL `SHOW REPLICA STATUS`) | `{}` |
| `monitor.alerts` | **closes the Navicat id** — threshold rules on the existing monitor data, evaluated on the job scheduler | `{"rules":[{"metric":"connections","op":">","value":100}]}` |

## Design

### One job runner, used by everything slow

A new module `crates/engine/src/jobs_async.rs` (not to be confused with `jobs.rs`, the scheduler — name it
`runner.rs`):

```rust
pub struct JobId(u64);
pub struct Job { pub state: JobState, pub started: Instant, pub cancel: Arc<AtomicBool>, pub rows: usize }

pub enum JobState { Running, Done(ResultSet), Failed(String), Cancelled }
```

`Session` gains `runner: Runner`, which owns `HashMap<JobId, Job>` plus a worker per job. The worker owns a
**second connection** — this is the part that makes cancellation possible at all: cancelling a query requires
issuing `pg_cancel_backend(pid)` from a *different* session, and the driver's single connection is busy
executing. So:

- `sql.runAsync` opens a sibling connection from the same profile (`dial_config`, same tunnel), records the
  backend pid, and runs the statement there.
- `sql.cancel` connects a third time (cheap — it is one statement) and calls `pg_cancel_backend(pid)` /
  `KILL QUERY <id>`. For SQLite, which has no out-of-band cancel, the flag is checked between pages and the
  command says `{"cancelled":"at next page boundary"}` — honest about the granularity.
- The UI polls `sql.jobStatus` each frame (the `ai.rs` pattern) and paints a spinner plus a live row count.

**Streaming into the register.** The worker sends pages over the `mpsc` channel as it reads them, and the frame
appends them to the tab's `StatementRun`. That is what makes "results start appearing immediately" true without
a second result type: a `StatementRun` in progress is just a `ResultSet` that is still growing, with
`SqlState` carrying `running: Option<JobId>` so the register knows to paint the in-progress state.

Ceiling to record: the frame appends, so a 10-million-row async run still costs memory per row. `max_rows`
still bounds it; real streaming-to-disk is phase 5's `export.stream`.

### Explain plans become a graph from the text the server already returns

`sql.explain` returns `EXPLAIN (FORMAT JSON)` output? Check first: `cmd/sql.rs::explain` builds
`EXPLAIN`/`EXPLAIN ANALYZE` with no format clause, so it gets text. Text plan parsing is brittle. So step one
of this item is: ask for JSON where the engine supports it (Postgres `FORMAT JSON`, MySQL `FORMAT=JSON`),
keep text for SQLite (whose `EXPLAIN QUERY PLAN` output is a flat list with parent ids, which is enough).

A normaliser in `crates/model/src/plan.rs`:

```rust
pub struct PlanNode { pub id: usize, pub label: String, pub detail: String, pub cost: f64,
                      pub rows: f64, pub actual_ms: Option<f64>, pub children: Vec<usize> }
pub struct Plan { pub nodes: Vec<PlanNode>, pub root: usize }
```

Postgres JSON maps to this almost directly (`Plan` → node, `Plans` → children, `Total Cost`, `Actual Total
Time`). MySQL's JSON has a different shape (`query_block` → `nested_loop` → …) and gets its own mapping
function; SQLite's rows are `(id, parent, notused, detail)` and map trivially.

The UI draws the tree with the existing `theme` tokens: one box per node, indented children connected by
elbows, cost as a bar, and the node with the largest `cost` share highlighted (that is the one a user cares
about). `sql.planVisualiser` is the command that hands the UI the normalised `Plan`; the UI does no parsing.

### The profiler is a loop, and it is honest about variance

`sql.profiler` runs the statement `runs` times (default 5, capped at 50), collects the elapsed time and the
plan per run, and returns `{"min","median","max","mean","runs":[{elapsed_ms, plan}]}`. Rules:

- A statement whose first word is not `SELECT`, `WITH`, `EXPLAIN` or `SHOW` is refused — profiling an `UPDATE`
  fifty times is a data-integrity incident, not a measurement.
- `EXPLAIN ANALYZE` *executes* the statement, so `analyze: true` on a write statement is refused for the same
  reason.
- The report says whether the runs varied more than 20%, because a profiler that hides variance teaches the
  wrong lesson.

### Locks, replication and alerts: read-only queries, one shape

- `monitor.locks` — Postgres: `pg_blocking_pids()` joined to `pg_stat_activity`; MySQL:
  `performance_schema.data_lock_waits` joined to `data_locks` (8.0+) with a fallback to
  `information_schema.INNODB_TRX` for older servers. Returns `[{blocked_pid, blocking_pid, wait_s, query,
  blocked_query}]`. SQLite returns an empty list with the note that it has no server-level locks.
- `monitor.replication` — Postgres `pg_stat_replication` (`state`, `replay_lag`); MySQL `SHOW REPLICA STATUS`
  mapped to the same fields; SQLite empty with a note.
- `monitor.alerts` — rules stored in the store, evaluated by the existing job scheduler (`jobs.rs`) on a
  cadence, firing the existing `job.notifications` webhook. **Reuse, do not add a second notification path.**
  A rule that trips logs at `Level::Warn` and POSTs the same JSON shape a job failure does.

All three are read-only and must degrade to `Error::Unsupported` with a sentence on an engine that has no such
concept — never an empty table that reads as "nothing is wrong".

## Files

| File | Work |
|---|---|
| `crates/engine/src/runner.rs` (new) | `JobId`, `Job`, `Runner`, the worker, the cancel path |
| `crates/engine/src/lib.rs` | `Session.runner`, `SqlState.running`, `StatementRun` growth |
| `crates/engine/src/cmd/sql.rs` | `sql.runAsync`, `sql.jobStatus`, `sql.cancel`, `sql.stop`, `sql.profiler`, `sql.explainVisual` |
| `crates/drivers/src/lib.rs` | `Driver::cancel(pid)`, `Driver::backend_pid()`, `Driver::run_script_streaming(…)` |
| `crates/drivers/src/{pg,mysql,sqlite}.rs` | the three implementations; `explain_json` |
| `crates/model/src/plan.rs` (new) | `Plan`, `PlanNode`, the three normalisers |
| `crates/engine/src/cmd/monitor.rs` | `locks`, `replication`, `alerts` |
| `crates/ui/src/register.rs` | in-progress state: spinner, live row count, Stop enabled |
| `crates/ui/src/views.rs` | the plan graph |
| `crates/ui/src/app.rs` | poll running jobs each frame |
| `crates/ui/src/dialogs.rs` | alert rules editor |

## Steps

1. `runner.rs` + `sql.runAsync` for SQLite only (no cancel), with the frame polling and the register growing.
   This proves the shape without touching driver cancellation.
2. `Driver::backend_pid` / `cancel` for Postgres, then MySQL; `sql.cancel`; `sql.stop` becomes live.
3. Async runs for every command that currently blocks: `sql.run`, `grid.fetchPage` on a slow filter,
   `grid.countRows`, `tools.importCsv`, `transfer.data`. **One at a time, each with its own test** — this is the
   phase's risk, and a big-bang rewrite of every call site is how it fails.
4. `explain_json` per driver + `model/src/plan.rs` normalisers + `sql.explainVisual`.
5. The plan graph in the UI.
6. `sql.profiler`.
7. `monitor.locks`, `monitor.replication`.
8. `monitor.alerts` over the job scheduler.
9. Regenerate parity.

## Tests

- Runner: an async run completes and lands in the tab; the frame does not block (assert the poll returns
  `Running` at least once before `Done`).
- Cancel: a `pg_sleep(30)`-style query is cancelled in under a second and the tab reports `Cancelled`, not
  `Failed`.
- Cancel granularity: SQLite reports the page-boundary wording.
- Normalisers: a captured Postgres JSON plan, a MySQL JSON plan and a SQLite `EXPLAIN QUERY PLAN` list each
  produce the expected node count, root and the highest-cost node.
- Profiler: a write statement is refused; 5 runs return 5 timings and a median within the min/max.
- Locks: two connections, one holding a row lock, the other blocking → `monitor.locks` names both pids.
- Alerts: a rule that trips calls the same webhook path a failed job does (assert the JSON shape).
- Unsupported: `monitor.replication` on SQLite is `Error::Unsupported`, not an empty list.
- Sweep covers the eleven new ids.

## Done when

A query against a slow table paints rows as they arrive, Stop actually stops it, and a plan shows up as a tree
with the expensive node marked.

## Ceilings

- **Async is opt-in per command**, applied one call site at a time. Not every blocking path is converted in
  this phase; the ones left are named in `docs/gaps.md` when it closes.
- **Results still accumulate in memory**, bounded by `max_rows`. Disk streaming is `export.stream`.
- **Alerts evaluate on a schedule**, not continuously, and only on the metrics already collected.
- **The plan graph is a tree, not a flame graph.** No cost heat-map across edges.
- **SQLite has no cancel**; it stops at a page boundary and says so.
