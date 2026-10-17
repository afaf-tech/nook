# Feature inventory — Beekeeper Studio and TablePlus

Harvested 2026-10-17 from `beekeeperstudio.io/features` and its eight subpages
(`easy-to-use`, `sql-editor`, `sql-ai`, `table-data`, `json-sidebar`, `alter-schemas`,
`erd-schema-viewer`, `import-export`, `query-magics`, `workspace`), and from `tableplus.com` plus
`docs.tableplus.com` (its `llms.txt` index, and the pages for the interface, quick look, right
sidebar, filter, open anything, code review and safe mode, query editor, streaming results, split
panes, query parameters, query history, keyword binding, metrics board).

Every row carries a verdict and the evidence. **Verdicts:** `have` (exists today, id named),
`gap` (missing, phase named), `reject` (deliberately out of scope, reason given).

Verdicts are claims about *this* checkout. Where a row says `have`, the command id or file exists;
where it says `gap`, the absence was grepped for, not assumed.

## The headline

| | Beekeeper Studio | TablePlus |
|---|---|---|
| What it is | Electron, open-core, "easy to use" positioning | Native macOS/Windows, "fast and safe" positioning |
| Its real edge over a Navicat-shaped tool | global quick search, JSON sidebar, query magics, editable query results, streaming import/export with upsert | row filters UI, right sidebar bulk edit, safe mode, split panes, query params, metrics board |
| Its weakness Nook already beats | no CLI, no MCP, no control channel; engine breadth is a paid tier | same — every feature is GUI-only, scripting is an afterthought |

The pattern in both: **the engine features are table stakes and Nook mostly has them; the daily-friction
features are where they win.** Neither app can be driven by a script. That is the thing this plan
protects while closing the friction.

---

## 1. Connections

| Feature | Source | Verdict | Evidence / phase |
|---|---|---|---|
| SSL connections | both | `have` | `conn.setSsl`, five modes |
| SSH tunnel | both | `have` | `conn.setSshTunnel`, shells out to `ssh` so agent keys work |
| SSH by password | Nook | `have` | `conn.sshPassword` |
| Database URL parsing | both | `have` | `conn.parseUri` |
| Double-click a SQLite file to open | Beekeeper | `have` | `nook file.db`, `sqlite:` URI |
| Multiple databases per connection | both | `have` | `conn.databases`, `conn.useDatabase` |
| Connection groups / folders | Beekeeper | `have` | `conn.groups`, `ConnectionConfig.group`, painted by `tree.rs:405` |
| Drag-and-drop folder reordering | Beekeeper | `gap` | 1 — pins and order, not drag |
| Pinned / unpinned items with persistent state | Beekeeper | `gap` | 1 |
| HTTP tunnel | Nook catalog | `gap` | 9 (platform), lowest value of the three tunnels |
| Cloud database connections | Nook catalog | `reject` | A hosted broker; phase 9 ships exportable config instead |
| Cloud workspace sync, team folders, share links | Beekeeper | `reject` | Needs a server and an account model. Local half lands in 1 and 9 |
| Connection colour painted in the tree | Beekeeper-ish | `gap` | 1 — the field exists (`ConnectionConfig.color`), nothing paints it |
| DBngin (local server manager) | TablePlus | `reject` | Not a database client feature; users run their own servers |
| CockroachDB JWT auth | Beekeeper | `reject` | No CockroachDB driver |
| BedrockDB, ScyllaDB, Greengage, DuckDB | Beekeeper | `reject` | Engine breadth is phase 9's `conn.*` story for SQL Server/Oracle/Mongo/Redis, not these |
| Plugin system, plugin admin controls | both | `reject` | A plugin ABI is a second product. The command registry + MCP is the extension story |

## 2. Browsing and finding

| Feature | Source | Verdict | Evidence / phase |
|---|---|---|---|
| Object tree with schemas, tables, views | both | `have` | `tree.rs`, catalog |
| Tree filter (name contains) | both | `have` | `tree.rs:191` |
| **Global quick search — `Cmd+P`, fuzzy, tables + views + saved queries** | both | `gap` | **2** — the `Cmd+K` palette searches *commands* only (`dialogs.rs::palette`); nothing opens an object by name |
| Clickable foreign keys — jump to the referenced row in a new tab | both | `gap` | 3 |
| Quick Look — a cell in a bigger popup, JSON and BLOBs pretty-printed | TablePlus | `gap` | 3 |
| JSON sidebar — a row as nested JSON, FKs expanded in place, search + regex | Beekeeper | `gap` | 3 |
| Right sidebar — the selected row as a vertical form, editable, column search | TablePlus | `have` | `grid.formView` + the form band. Bulk edit across several rows is `gap` → 4 |
| Structure view for a table (columns, indexes, relations, triggers, partitions) | Beekeeper | `have` | Design tab: `schema.columns`, `schema.indexes`, `schema.foreignKeys`, `schema.ddl`; triggers via `schema.triggerEditor`. Partitions → 7 |
| Database objects: functions, procedures, views | TablePlus | `gap` | 7 |
| Server-wide search | Nook catalog | `gap` | 2 — the quick-open index *is* this |

## 3. The data grid

| Feature | Source | Verdict | Evidence / phase |
|---|---|---|---|
| Paging, sort, filter | both | `have` | `grid.fetchPage`, `grid.sort`, `grid.filter` |
| **Row filters as a UI — `Cmd+F`, several filters, per-filter apply, SQL preview** | TablePlus | `gap` | **4** — today the filter bar takes one raw `WHERE` |
| Column filter — show only chosen columns | TablePlus | `gap` | 3 |
| Hide useless columns | Beekeeper | `gap` | 3 (same mechanism, per-table persistence) |
| Inline typed editing | both | `have` | `grid.editCell` with type coercion |
| Staged changes, revert, copy to SQL | both | `have` | `grid.pending`, `grid.revert`, `grid.copyAsSql` |
| Apply in one transaction | both | `have` | `grid.apply` |
| Bulk edit across selected rows | TablePlus | `gap` | 4 |
| Multi-cell selection, `Cmd+C` as a block | both | `gap` | 3 — selection is one cell (`table.rs:53 Option<(usize, usize)>`) |
| Copy as an Excel-formatted table (TSV) | Beekeeper | `gap` | 3 |
| Blob editor, text and hex | Nook catalog | `gap` | 3 |
| Edit the SQL of a row (`rawMode`) | Nook catalog | `gap` | 4 |
| Query Magics — alias-driven formatting (`__format__stars`, `progress`, `check`, `money`, `link`, `image`, `__goto__`, `__format__enum`) | Beekeeper | `gap` | 3, stretch: `check`/`money`/`link` only; `enums.json` rejected |

## 4. The query editor

| Feature | Source | Verdict | Evidence / phase |
|---|---|---|---|
| Multi-statement scripts, run all, run statement, run selection | both | `have` | `sql.run`, `sql.runStatement`, `sql.runSelection` |
| One result tab per statement | TablePlus | `have` | `SqlState::results`, the register |
| Syntax highlighting | both | `have` | `editor.rs::sql_job` |
| Autocomplete from the live catalog | both | `have` | `sql.complete` |
| Format / beautify | both | `have` | `sql.format` |
| Query history | both | `have` | `sql.history` — **in memory only**, lost on exit → 1 |
| Saved queries / favourites / keyword binding | both | `gap` | `snippet.save` exists but **is not persisted** and `sql.snippets` has no command → 1 |
| Zen mode (editor full-bleed) | Beekeeper | `gap` | 6 |
| Split panes — two editors side by side, `Cmd+Shift+D` | TablePlus | `gap` | 6 |
| Find & replace in the editor | Beekeeper | `gap` | 6 |
| Multiple carets | TablePlus | `gap` | 6, stretch |
| **Query parameters** — `:name`, `%name%`, `?`, `${name}`, with a prompt for values | TablePlus | `gap` | **6** — `sql.parameterised` is catalogued as not implemented |
| Streaming results, async loading — the UI keeps working while rows arrive | TablePlus | `gap` | **8** — `sql.run` blocks the frame |
| Stop a running statement | Nook catalog | `gap` | 8 — the button exists, painted disabled, honest |
| Explain, `EXPLAIN ANALYZE` | both | `have` | `sql.explain` |
| Visual explain plan | Nook catalog | `gap` | 8 |
| Query profiler | Nook catalog | `gap` | 8 |
| Editable query results — edit a `SELECT`'s output, mapped back to its source table | Beekeeper | `gap` | 4 — the register is read-only by design (`register.rs READ_ONLY`) |
| AI shell / LLM plugin — prompt to SQL | both | `have` | `ai.generate`, 7 providers, `@table` context. A chat transcript is `reject` |

## 5. Design and schema

| Feature | Source | Verdict | Evidence / phase |
|---|---|---|---|
| No-code table creator | both | `have` | `schema.createTable` |
| Alter columns — type, nullability, default | both | `have` | `schema.changeColumnType`, `setNullable`, `setDefault` |
| Create/drop indexes, composite indexes | both | `have` | `schema.createIndex`, `dropIndex` |
| Create/drop foreign keys | both | `have` | `schema.addForeignKey`, drop via DDL |
| Auto-increment primary key helper | Beekeeper | `have` | `schema.createTable` `auto_increment` |
| Copy to SQL — generate DDL into a new editor tab | Beekeeper | `have` | `schema.ddl` |
| Views | both | `have` | `schema.createView`, `dropView` |
| Trigger, function, sequence editors | TablePlus, Nook catalog | `have` | `schema.triggerEditor` / `functionEditor` / `sequenceEditor` (shipped in `014a124`) |
| PostgreSQL partitions | Beekeeper | `gap` | 7 |
| Rebuild a table in place (SQLite) | Nook catalog | `gap` | 7 — the driver can (`rebuild_table`, used by `schema.setPrimaryKey`); `schema.tableRebuild` is not exposed |
| Table rename / copy / truncate / empty database | Nook | `have` | `schema.*` |
| ER diagram from the live catalog, pan/zoom, SVG export | both | `have` | `diagram.erDiagram` |
| **ERD export as PNG, copy to clipboard, hand-arranged layout, save/load a model file** | both | `gap` | **7** |
| Forward / reverse engineering | Nook catalog | `have` (reverse), `gap` (forward → 7) |
| Data dictionary export | Nook catalog | `gap` | 7 |

## 6. Import and export

| Feature | Source | Verdict | Evidence / phase |
|---|---|---|---|
| Export grid / table / query to CSV, JSON, SQL, Markdown, Excel | all three | `have` | `grid.export`, `grid.exportTable`, `tools.exportQuery`, `tools.exportExcel` |
| Copy result as an Excel-pasteable table | Beekeeper | `gap` | 3 |
| **Streaming export — page a giant result straight to a file** | Beekeeper | `gap` | **5** — `run_sql` is bounded by `max_rows`, so a big export is truncated or refused |
| **Streaming import — read a giant CSV without loading it** | Beekeeper | `gap` | **5** — `import_csv` does `read_to_string` |
| **Import: append vs truncate-first** | Beekeeper | `gap` | **5** |
| **Import: upsert on duplicate key** | Beekeeper | `gap` | **5** |
| Import: automatic column mapping and type detection | Beekeeper | `have` (mapping), `gap` (auto-detect) | `columnMapping` exists, name-based; auto-detect lands in 5 |
| Import wizard UI with preview and a mapping editor | Nook catalog | `gap` | 5 — parameter-level today |
| Import CSV / JSON / Excel / `.sql` | all three | `have` | `tools.import*` |
| Import XML, DBF, text-file-as-table | Nook catalog | XML and fixed-width text → 5; DBF `reject` (dead format) |
| Data generation | Nook | `have` | `tools.datagen`, 20 generators |
| Transfer between connections, cross-engine | Nook | `have` | `transfer.data`, `transfer.preview` |
| Transfer column mapping, transfer speed | Nook catalog | `have` (name mapping via `columnMapping`), `gap` (speed test → 5) |

## 7. Safety and review

| Feature | Source | Verdict | Evidence / phase |
|---|---|---|---|
| Staged changes before commit | both | `have` | pending list |
| **Code preview — see the exact SQL a pending change will run** | TablePlus | `have` | `grid.pending`, the pending band |
| Commit / discard changes | TablePlus | `have` | `grid.apply`, `grid.revert` |
| **Safe mode — re-authenticate before destructive actions** | TablePlus | `gap` | **4** — `confirm_destructive` is a boolean today, no second factor |
| Destructive ops require `confirm: true` | Nook | `have` | engine-level, `docs/conventions.md` |
| Read-only connection | Nook | `have` | `conn.setReadOnly`, refused before the server sees it |
| Manual commit mode (transaction held open) | both | `have` | `sql.transaction`, `sql.runInTransaction` |

## 8. Everything else both apps ship

| Feature | Source | Verdict | Evidence / phase |
|---|---|---|---|
| **Tabs that survive a restart** | Beekeeper | `gap` | **1** — `Session::new` always starts on a Welcome tab |
| Multi-window | Beekeeper | `gap` | 9 stretch — `egui` multi-viewport; single `ViewportBuilder` today |
| Backup and restore (`.sql`, `pg_dump`, `mysqldump`) | both | `have` | `backup.dump`, `backup.restore`, `backup.list` |
| Compressed / encrypted backups, and a kept local copy | Nook catalog | `gap` | 5 (`backup.compression`, `backup.encryption`, `backup.local`) |
| Row sync, structure diff and structure sync | Nook | `have` | `sync.data`, `sync.structure`, `diff.structure` |
| Delete rows missing from the source, conflict rules, bi-directional sync | Nook catalog | `gap` | 5 |
| Data profiling and a data dictionary export | Nook catalog | `gap` | `tools.dataProfiling` → 9, `tools.dataDictionary` → 7 |
| Compressed / encrypted backups | Nook catalog | `gap` | 5 |
| User management, grants | TablePlus | `have` | `priv.*` |
| Role membership editor, privilege audit | Nook catalog | `gap` | 9 |
| Metrics board — several charts, refresh rate, input fields driving query params | TablePlus | `gap` | **9** |
| Report designer, BI dashboard, report export | Nook catalog | `gap` | 9 |
| Server monitor — processes, kill, status, variables | both | `have` | `monitor.*` |
| Locks, deadlocks, replication, threshold alerts | Nook catalog | `gap` | 8 |
| Scheduler / jobs, chaining, failure webhook | Nook | `have` | `job.*` |
| Deploy jobs to another machine, job export/import | Nook catalog | `gap` | 9 |
| Localisation | Nook catalog | `gap` | 9 stretch — Indonesian first, the user is in Balikpapan |
| Windows and Linux builds | Nook catalog | `gap` | 9 — CI matrix, not code |
| Auto-update | Nook catalog | `reject` | Distribution concern; `install.sh` is the channel today |

---

## Rejected

Collected so they are answered once rather than re-litigated per phase.

| Thing | Why not |
|---|---|
| Cloud workspaces, team folders, real-time folder sync, share links, per-field AES-GCM | Server, accounts, and a trust boundary that does not exist in a local tool. The local half — pins, folders, an exportable query library — is phases 1 and 9. |
| Plugin system with admin controls | An ABI, a sandbox and a marketplace. `CommandSpec` + MCP already let anything script the app; a plugin API would be a second, weaker one. |
| A second AI surface (chat shell, embeddings) | `ai.generate` covers prompt-to-SQL over seven providers with `@table` context. A transcript adds surface, not capability. |
| Query Magics' `enums.json` indirection | A side-channel config file for what a `CASE` expression or a lookup table already does in SQL. The cheap formatting half is phase 3. |
| DBngin, an in-app updater, CockroachDB JWT | Not client features, or not our engines, or a distribution problem. |
| DBF import | A format whose users have already migrated. XML and fixed-width text are phase 5. |
| Engines: BedrockDB, ScyllaDB, Greengage, DuckDB | Phase 9 spends the engine budget on SQL Server, Oracle, MongoDB and Redis — the four already catalogued with a `Driver` seam waiting. |

## Where this inventory disagrees with the Navicat catalog

`crates/engine/src/catalog.rs` is measured against Navicat. Several entries here have **no Navicat
equivalent**, so they cannot move the parity number and must not be forced into a catalog id that
means something else:

`quickOpen`, `grid.hideColumns`, `grid.copyTsv`, `grid.quickLook`, `grid.rowJson`, `grid.followForeignKey`,
`grid.selection`, `grid.bulkEdit`, `grid.filters` (the UI over `grid.filter`), `sql.splitPane`,
`sql.zen`, `sql.findReplace`, `sql.multiCaret`, `editor.safeMode`, `report.metricsBoard`,
`import.upsert`, `import.truncate`, `import.stream`, `export.stream`, `state.persist`,
`snapshot.json`.

Two consequences for the parity gate:

1. Those ids appear in the report's `extra` list ("commands beyond the catalog") — `ParityReport::extra`
   already counts them (`crates/engine/src/parity.rs:30`), `summary()` prints the number, and the
   markdown section `## Beyond the catalog` (`parity.rs:106`) names each one in
   `docs/parity-checklist.md`. No further work is needed to make the extras auditable.
2. The Navicat ids this plan *does* close are named per phase, so the coverage percentage moves for a
   reason a reader can check. The floor in `parity.rs` rises at the end of each phase that closes one.
