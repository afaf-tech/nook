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

## 4. ~~Trigger, function and sequence editors~~ — done

Shipped: `schema.triggerEditor` / `schema.functionEditor` / `schema.sequenceEditor`, each with
`action` list/create/drop. Lists read the catalog cache; create builds the `CREATE TRIGGER` /
routine / `CREATE SEQUENCE` per dialect and runs it as one verbatim statement (so trigger and
routine bodies containing `;` survive), then refreshes the catalog. Drop asks for `confirm:true`.
PG `DROP TRIGGER` gets the unqualified name plus `ON <qualified table>`. Guarded at the command
level: routines refuse on SQLite, sequences only exist on PostgreSQL. Catalog introspection fills
triggers/routines/sequences for PostgreSQL (`pg_trigger`/`pg_proc`/`pg_sequences`) and triggers/
routines for MySQL (`information_schema`); MySQL routine DDL stays empty rather than one `SHOW
CREATE` round trip per object. The design toolbar gains `+ Trigger` / `+ Function` / `+ Sequence`.
Still open: a visual editor surface — the commands are parameter-level today, like the rest of the
command palette.

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

## 7. ~~Cancelling a running statement~~ — engine done, UI wiring open

Shipped: `sql.runAsync` starts a run on a worker and reports pages as they land; `sql.stop` /
`sql.cancel` cancel it. The worker dials its own connection and records the backend pid, so a cancel
dials a sibling connection from the same profile and issues `pg_cancel_backend` (Postgres) or
`KILL QUERY` (MySQL). SQLite has no server-side cancel: it stops between pages and the command's
answer says exactly that. A cancelled run's state is `cancelled`, never `failed`.

Still open: the editor's Run button still calls blocking `sql.run`, so the design's `Executing…` pill
and `.prog` hairline are still not painted. Converting the remaining call sites to `sql.runAsync` and
polling `sql.jobStatus` per frame is the wiring, not the mechanism.

## 8. ~~Visual explain plans and a profiler~~ — done

Shipped: `sql.explainVisual` normalises each engine's own plan into a tree (`FORMAT JSON` on Postgres
and MySQL, `(id, parent, detail)` rows on SQLite), flags the most expensive node, and refuses
`analyze` on a statement that would write. The register's third view draws it. `sql.profiler` runs a
read `n` times (capped at 50), reports min/median/max and the spread, and refuses a write before it
runs. `monitor.locks` and `monitor.replication` read each server's own views and refuse on SQLite
instead of returning an empty table that reads as "nothing is blocked". `monitor.alerts` evaluates
rules against `monitor.status` metrics and refuses a metric name it does not have.

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
