# Nook gaps

The ranked work list, most valuable first. Every entry names the catalog id it would close, so the
parity number moves when it lands.

## 1. ~~Verify MySQL against a live server~~ — done

The local Homebrew `mysqld` was unusable (8.3.0_1 links `icu4c.74`, only 78 ships — dyld abort before
start), and reinstalling it would replace the user's server, so it was left untouched. Instead a
portable `mysql-8.4.3-macos14-arm64` tarball was extracted to `/private/tmp` and run on port 3307
with its own datadir. Against it the battery ran end to end: `live-check.sh` (connection, catalog,
monitor, users, grid edits, dump via `mysqldump`, restore, diagram, chart), `transfer` (`2 rows,
0 failed`), `sync-data --apply` (`1 applied`, verified by reading the rows back), and
`diff-structure` (`0 change(s)`).

The battery surfaced two bugs, both fixed test-first: cross-driver structure diffs matched schemas by
name (SQLite `main` never equals the MySQL database name) and emitted a spurious add+drop pair; and
`SyncReport` labeled any run with zero applied statements "(dry run)", including applied syncs over
already-in-sync tables. Both carry regression tests; evidence table in
`.agile/nook/verification-2026-10-16-173802-nook.md`.

## 2. ~~Form view for one record~~ — done

Shipped: the engine command (`grid.formView`) and the UI form band, including editable fields that
go through the same `grid.editCell` / pending-changes path as the grid.

## 3. ~~Column mapping in the import wizard and the transfer~~ — done

Shipped: name mapping via `columnMapping` (`src=target` pairs) for CSV/JSON imports in
`tools.importCsv`/`tools.importJson` and for `transfer.data`, validated against the destination
catalog; mapping refuses `tables: ["*"]`. Still open: the import wizard UI does not surface a
mapping editor — it is parameter-level today. (`tools.importWizard`, `transfer.columnMapping`)

## 4. Trigger, function and sequence editors

Object design stops at tables, views and indexes. Triggers and stored routines are read-only today —
they appear in DDL but cannot be created or edited. (`schema.triggerEditor`, `schema.functionEditor`,
`schema.sequenceEditor`)

## 5. ~~Job notifications and chaining~~ — done

Shipped: `job.notifications` lists, sets and clears a failure webhook (the job's JSON status
POSTed via the system curl — email is a webhook to a mail relay), and chains are editable after
creation via `job.chain` (list or set `then`, validated acyclic, empty array clears; `job.create`
accepts the same field). A chained failure stops the chain, a due chained child never runs twice
in one tick, a referenced job cannot be deleted, and a hand-edited cycle is stopped at run time.
Still open: `job.agent`, deploying jobs to another machine.

## 6. ~~Excel import and export~~ — done

Shipped: `tools.importExcel` reads the first sheet of an `.xlsx` (calamine) through the same
`load_rows` path as CSV — `columnMapping`, `create_table` and `max_rows` behave identically — and
`tools.exportExcel` writes a query result as a plain one-sheet workbook. Still open: styles,
multi-sheet output and `.xls` (legacy binary) input.

## 7. Cancelling a running statement

A long query cannot be stopped from the app: each run is bounded by `max_rows`, which limits rows but
not time. Needs a driver-level cancel (Postgres `pg_cancel_backend` on the same backend, MySQL `KILL
QUERY`). (`sql.stop`)

The UI's half is built: the editor's `Stop` control exists and is painted disabled, and its hover text
is the command registry's own `sql.stop` help, so the button cannot claim more than the engine can do.
`sql.run` blocks the UI thread, which is why no frame claims a statement is running. The ceiling is a
job id the UI can poll plus a driver-level cancel, at which point `Stop` becomes a live button.

## 8. Visual explain plans and a profiler

`EXPLAIN` output is tabular. A plan graph and a per-query profile are what Navicat's monitor tab
offers next. (`sql.planVisualiser`, `sql.profiler`, `monitor.locks`, `monitor.replication`)

## 9. More engines

SQL Server, Oracle, MongoDB and Redis are in the catalog with no driver. The `Driver` trait is the
seam: a new module plus introspection queries. (`conn.sqlserver`, `conn.oracle`, `conn.mongodb`,
`conn.redis`)

## 10. Report designer and BI dashboard

Charts are live views over a query. A saved report with a layout, and a dashboard that shows several
of them at once, is a different feature. (`report.designer`, `report.biDashboard`)

## 11. Compressed and encrypted backups

The vendor tools write plain files. `gzip` plus an age or GPG wrapper is the shape. (`backup.compression`,
`backup.encryption`)

## 12. Smaller, sharper items

- Copy a view definition when synchronizing structures (`diff` reports views as manual today).
- Bi-directional and rule-based sync (`sync.bidirectional`, `sync.conflictRules`).
- Blob editing: text and hex editors for `bytea`/`blob` columns (`grid.blobEditor`).
- Parameterised queries from the editor (`sql.parameterised`).
- An HTTP tunnel for locked-down networks (`conn.httpTunnel`). SSH-by-password is built
  (`conn.sshPassword`); it needs the login keychain, and `docs/architecture.md` records what happens
  without one.
- Data profiling and a data dictionary export (`tools.dataProfiling`, `tools.dataDictionary`).
- Localisation (`platform.i18n`), Windows and Linux builds (`platform.windows`, `platform.linux`).
- Cloud and team features: shared query library, synced settings (`cloud.*`).
- A modelled-layout editor: move boxes by hand, save and load a model file (`diagram.manualLayout`,
  `diagram.modelFile`).

## Known quality ceilings, not gaps

These are deliberate and documented where they live (see `docs/architecture.md`):

- PostgreSQL uses the simple query protocol: exact `numeric` text, no bind parameters yet.
- Edits, imports, transfers and dumps build client-side literals through `sql_literal`.
- PostgreSQL DDL is generated from the catalog rather than asked of the server.
- The ER layout is deterministic; there is no physics or manual arrangement.
