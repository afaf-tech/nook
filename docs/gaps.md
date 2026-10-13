# DataCraft gaps

The ranked work list, most valuable first. Every entry names the catalog id it would close, so the
parity number moves when it lands.

## 1. Verify MySQL against a live server

The driver is written and unit-tested, but no MySQL server was reachable on this machine: the local
Homebrew `mysql` (8.3.0_1) links `icu4c.74`, and Homebrew no longer ships that version (it offers 75,
76 and 78), so `mysqld` aborts before it starts:

```
dyld: Library not loaded: /opt/homebrew/opt/icu4c/lib/libicuuc.74.dylib
Referenced from: /opt/homebrew/Cellar/mysql/8.3.0_1/bin/mysqld
```

Reinstalling MySQL would replace the user's server, so it was left alone. Everything
MySQL-specific — the `information_schema` joins, `SHOW CREATE TABLE`, `USE`, `KILL`, the boolean and
date shapes, `mysql.user` — is therefore unexercised end to end.

*Fix:* any reachable MySQL 8 or MariaDB (a container, a remote server, or a repaired local install),
then run the battery:

```sh
scripts/live-check.sh mysql://root@127.0.0.1/shop
datacraft-cli transfer --from sqlite:./shop.db --to mysql://root@127.0.0.1/stage --tables '*'
datacraft-cli sync-data --from sqlite:./shop.db --to mysql://root@127.0.0.1/stage --apply
datacraft-cli diff-structure --from sqlite:./shop.db --to mysql://root@127.0.0.1/stage
```

## 2. Form view for one record

Navicat's form view edits a single row as a vertical form, which is the sane way to work with wide
tables. The data grid covers everything else. (`grid.formView`)

## 3. Column mapping in the import wizard and the transfer

Both map columns by position. Real files have headers in a different order and tables get renamed
columns. (`tools.importWizard`, `transfer.columnMapping`)

## 4. Trigger, function and sequence editors

Object design stops at tables, views and indexes. Triggers and stored routines are read-only today —
they appear in DDL but cannot be created or edited. (`schema.triggerEditor`, `schema.functionEditor`,
`schema.sequenceEditor`)

## 5. Job notifications and chaining

A failing nightly backup currently only shows in the log. A webhook or email hook plus "run this job
after that one" is what makes the scheduler useful unattended. (`job.notifications`, `job.chain`,
`job.agent`)

## 6. Excel import and export

CSV covers the technical case; spreadsheets are what people actually have. (`tools.exportExcel`,
`tools.importExcel`)

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
