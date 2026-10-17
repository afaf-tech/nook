# Phase 7 — Schema depth and the diagram

**Arc C · closes `schema.triggerEditor`, `schema.functionEditor`, `schema.sequenceEditor`, `schema.tableRebuild`, `diagram.reverseEngineer`, `diagram.forwardEngineer`, `diagram.manualLayout`, `diagram.modelFile`, `tools.dataDictionary` · depends on phase 3**

## The problem, measured

`e8c6041` added `Trigger`, `Routine` and `Sequence` to the model and `create_trigger`, `create_routine`,
`create_sequence` (with their `drop_*`) to `ddl.rs`. What it did **not** do is populate them from the server
or expose them:

| Fact | Evidence |
|---|---|
| Only the SQLite driver reports triggers | `drivers/src/sqlite.rs:239` reads `sqlite_master`; `grep -n "routines\|sequences\|triggers" drivers/src/pg.rs drivers/src/mysql.rs` → nothing |
| Postgres and MySQL report none of the three | the `Schema { name, tables }` assembly at `pg.rs:387` fills only tables |
| No command creates or edits any of them | `cmd/schema.rs` has no `trigger`/`routine`/`sequence` command |
| The DDL helpers are unreachable | `ddl::create_trigger` et al. exist and nothing calls them |
| No partition awareness beyond a relkind | `pg.rs:332` selects `relkind IN ('r','p','v','m','f')` — a partitioned table is listed, its partitions are not |
| No table rebuild | `schema.tableRebuild` catalogued as "SQLite DDL that needs it says so instead" |
| The diagram is one-way and un-editable | `diagram.erDiagram` + `diagram.exportSvg`; `layout_diagram` is deterministic, no positions are stored |
| No PNG, no model file, no data dictionary | `diagram.*` gap list in `docs/parity-checklist.md` |

This is the phase that makes "Object design 85%" true instead of papered over: today a trigger can be *read*
on SQLite and *nothing* on the other two, and cannot be created anywhere.

## Scope

| Command | What it does | Params |
|---|---|---|
| `schema.triggers` | **new** — list triggers for a table or schema | `{"table":"users"}` |
| `schema.createTrigger` / `schema.dropTrigger` | **new** — create and drop, gated on `confirm` for drop | `{"table":"users","name":"audit_ins","timing":"AFTER","events":["INSERT"],"body":"…"}` |
| `schema.triggerEditor` | **closes the Navicat id** — the editor's read/write surface: the DDL as text, validated, applied | `{"table":"users","name":"audit_ins","ddl":"CREATE TRIGGER …"}` |
| `schema.routines` | **new** — functions and procedures | `{"schema":"public"}` |
| `schema.createRoutine` / `schema.dropRoutine` | **new** | `{"name":"fn_total","kind":"function","args":"…","returns":"numeric","body":"…"}` |
| `schema.functionEditor` | **closes the Navicat id** | `{"name":"fn_total","ddl":"CREATE OR REPLACE FUNCTION …"}` |
| `schema.sequences` | **new** — list, with `last_value` | `{"schema":"public"}` |
| `schema.createSequence` / `schema.dropSequence` | **new** | `{"name":"order_no","start":1,"increment":1}` |
| `schema.sequenceEditor` | **closes the Navicat id** — edit start/increment/min/max/cycle (`ALTER SEQUENCE`) | `{"name":"order_no","increment":5}` |
| `schema.partitions` | **new** — a partitioned table's partitions, with bounds | `{"table":"events"}` |
| `schema.createPartition` / `schema.dropPartition` | **new** — Postgres `CREATE TABLE … PARTITION OF … FOR VALUES …`, MySQL `ALTER TABLE … ADD PARTITION` | `{"parent":"events","name":"events_2026_11","from":"2026-11-01","to":"2026-12-01"}` |
| `schema.tableRebuild` | **closes the Navicat id** — SQLite's 12-step rebuild, in one transaction, with a pre-flight check | `{"table":"users","changes":[…],"confirm":true}` |
| `diagram.reverseEngineer` | **closes the Navicat id** — the live catalog *is* the reverse-engineered model; this command returns it as a model document | `{"schema":"public"}` |
| `diagram.forwardEngineer` | **closes the Navicat id** — a model document to DDL, ordered by dependency | `{"model":{…}}` |
| `diagram.manualLayout` | **closes the Navicat id** — store box positions per `conn + schema` | `{"positions":{"users":[40,80]},"reset":false}` |
| `diagram.modelFile` | **closes the Navicat id** — save/load the model (positions included) as JSON | `{"path":"~/models/shop.json","action":"save"}` |
| `diagram.exportPng` | **new** — PNG alongside the SVG, via `nook --shot`'s existing renderer path | `{"path":"~/diagrams/schema.png"}` |
| `tools.dataDictionary` | **closes the Navicat id** — every table, column, type, key and comment as Markdown or HTML | `{"schema":"public","format":"markdown","path":"~/dict.md"}` |

## Design

### First, make the catalog tell the truth on all three engines

Nothing above works until `pg.rs` and `mysql.rs` populate the fields the model already has.

**PostgreSQL** (`pg_catalog`, one query each, added to the existing `catalog()` batch):

- triggers — `pg_trigger t JOIN pg_class c JOIN pg_namespace n`, `NOT t.tgisinternal`, with
  `pg_get_triggerdef(t.oid)` as the DDL and `t.tgenabled <> 'D'` as `enabled`.
- routines — `pg_proc p JOIN pg_namespace n`, `p.prokind IN ('f','p')`, `pg_get_functiondef(p.oid)` as the
  DDL, `pg_get_function_result(p.oid)` as `returns`.
- sequences — `pg_class c WHERE c.relkind = 'S'` with `pg_sequence` for start/increment/min/max/cycle and
  `pg_sequences.last_value`.

**MySQL** (`information_schema`):

- triggers — `information_schema.TRIGGERS`, `ACTION_TIMING`, `EVENT_MANIPULATION`, `ACTION_STATEMENT`; the DDL
  is assembled from those parts because MySQL stores no `CREATE TRIGGER` text.
- routines — `information_schema.ROUTINES`, `ROUTINE_TYPE`, `DTD_IDENTIFIER`, `ROUTINE_DEFINITION`.
- sequences — MySQL 8 has none; `AUTO_INCREMENT` metadata already covers the need (the model's own comment
  says so), so this returns an empty list with a note rather than pretending.

**SQLite** already reports triggers; its routines and sequences stay empty by design, and the UI says
"SQLite has no stored routines" rather than showing an empty panel that looks broken.

One test per driver, asserting a known trigger/routine/sequence created in the fixture comes back with the
right name, table and DDL text.

### The editors are DDL text with structure around it, not form builders

A form that builds `CREATE TRIGGER` covers the easy 60% and lies about the rest (MySQL's `FOR EACH ROW`,
Postgres's `WHEN` clause and `EXECUTE FUNCTION`, SQLite's `WHEN`/`BEGIN … END`). So the editor is:

- **Structured header** — name, table, timing, events (checkboxes), for Postgres a function reference. This is
  what makes the common case fast.
- **Body** — a monospace editor holding the statement body, pre-filled from `ddl::create_trigger` so the
  generated shape is visible and editable.
- **Preview** — the full statement that will run, always visible.
- **Apply** — the statement is sent verbatim. `create_trigger`'s structured form is used to *seed* the editor,
  never to regenerate over the user's edits.

Validation before sending: the statement must start with `CREATE` (or `ALTER`) and must mention the object
name. That is not a SQL parser and does not pretend to be — a typo still fails at the server, where the error
message is better than anything a client can produce. What the check catches is the one dangerous case: a
pasted statement for the wrong object, which would otherwise create something surprising.

`*Editor` commands take the whole DDL, so the same command serves "create" and "edit" — which is exactly what
`CREATE OR REPLACE` means on Postgres and what MySQL's `DROP`+`CREATE` dance needs behind the scenes. The
command says which it did in its result (`{"replaced": true}`), because MySQL's answer is destructive and
must not be silent.

### Partitions, narrowly

Read: Postgres `pg_inherits` + `pg_get_expr(relpartbound)` for the bounds; MySQL
`information_schema.PARTITIONS`. Write: range partitions by date only, which is the case anyone actually
uses — `CREATE TABLE events_2026_11 PARTITION OF events FOR VALUES FROM ('2026-11-01') TO ('2026-12-01')`.
List/hash partitions are the ceiling: refused with a message saying the DDL can be run in the query editor.
Postgres 17's default partition and MySQL's `REORGANIZE` are out of scope.

### SQLite's table rebuild, honestly gated

SQLite cannot drop or retype a column in place. The documented 12-step procedure is: `PRAGMA foreign_keys=off`,
begin, create the new table, copy, drop the old, rename, recreate indexes and triggers, `PRAGMA
foreign_key_check`, commit, `PRAGMA foreign_keys=on`.

`schema.tableRebuild` implements exactly that, with three refusals:

1. **Inside a transaction already** → refuse (the pragma cannot change inside one).
2. **`foreign_key_check` returns rows** → roll back and report them; a rebuild that breaks a reference is a
   data-integrity bug, not a warning.
3. **The table has a trigger or index whose DDL cannot be replayed** → refuse and list them, because silently
   dropping a trigger changes behaviour forever.

Today's behaviour ("SQLite DDL that needs it says so instead") stays for every other path; this command is the
one that does the work, and `schema.changeColumnType` on SQLite should call it rather than refusing.

### The diagram gets positions, a model file and a second export

`nook_model::layout_diagram` is deterministic. Add:

- `Diagram.boxes: Vec<(String, [f32; 2])>` — the layout's output, plus any stored override.
- `diagram.manualLayout` stores overrides in the store keyed by `conn + schema`; `reset: true` clears them.
- `diagram.modelFile` writes `{"version":1,"conn":"local","schema":"public","boxes":[…],"tables":[…]}` — the
  tables are included so a model file is readable without a live connection, which is what makes it a
  *document* rather than a bookmark.
- `diagram.forwardEngineer` takes that document and emits `ddl::create_table` per table in dependency order
  (topological sort over `foreign_keys`, cycles broken by emitting the FK as a separate `ALTER TABLE … ADD
  CONSTRAINT` after all tables exist — the only correct answer for a cycle).
- `diagram.exportPng` reuses the existing SVG rasterisation path the `--shot` flag already uses for the app;
  if that path is `eframe`-only, render the diagram offscreen through the same `egui` painter into a
  `ColorImage` and encode with the PNG writer already vendored for `--shot`. **Check which exists before
  writing code** — `grep -rn "png" crates apps` is the first command of this step, and if neither exists the
  item becomes "SVG only, PNG dropped", recorded as a ceiling rather than a silent failure.

### Data dictionary

`schema.ddl` already produces the DDL for every object; the dictionary is a different projection of the same
catalog: table → columns (name, type, nullability, default, key, comment), indexes, foreign keys, row estimate.
Markdown for a wiki, HTML for a share. One pass over `Catalog`, no queries beyond what is cached.

## Files

| File | Work |
|---|---|
| `crates/drivers/src/pg.rs` | triggers, routines, sequences, partitions |
| `crates/drivers/src/mysql.rs` | triggers, routines, partitions; sequences empty with a note |
| `crates/drivers/src/sqlite.rs` | keep triggers; make routines/sequences explicitly empty |
| `crates/engine/src/cmd/schema.rs` | the twelve schema commands |
| `crates/engine/src/cmd/diagram.rs` | reverse/forward engineer, PNG |
| `crates/model/src/diagram.rs` | `boxes`, overrides, the model document (serde) |
| `crates/model/src/ddl.rs` | `rebuild_plan` (the 12 steps), `create_partition`, dependency-ordered forward engineering |
| `crates/engine/src/store.rs` | diagram positions |
| `crates/ui/src/views.rs` | the Design tab's Triggers / Routines / Sequences / Partitions sections; the diagram canvas with drag |
| `crates/ui/src/dialogs.rs` | the object editor (header + body + preview) |
| `docs/architecture.md` | the rebuild gate, the MySQL replace-is-destructive note, the partition ceiling |

## Steps

1. Driver catalog work first, with a fixture per engine. No UI, no commands — this is the foundation and it is
   where the phase is most likely to be underestimated.
2. `schema.triggers` / `routines` / `sequences` / `partitions` (read-only) and the Design tab sections.
3. The object editor dialog + `schema.triggerEditor`, then `functionEditor`, then `sequenceEditor`.
4. `schema.tableRebuild` with the three refusals; make SQLite's `changeColumnType` route through it.
5. Partitions write path (range only).
6. Diagram: positions, drag, `manualLayout`, `modelFile`, `forwardEngineer`, then PNG (or the recorded ceiling).
7. `tools.dataDictionary`.
8. Regenerate parity — this phase should move several areas at once.

## Tests

- Per driver: a created trigger/routine/sequence appears in the catalog with the right DDL; `drop` removes it.
- Editor validation: a statement for a different object is refused; a valid one applies; MySQL's replace reports
  `replaced: true`.
- Rebuild: a table with a row, an index and a trigger survives with all three intact and the same row count; a
  rebuild that would orphan a reference is rolled back and the rows are listed; a rebuild inside a transaction
  is refused.
- Partitions: create a range partition, insert into the parent, and the row lands in the child (assert via the
  child's count); a list partition is refused with the DDL hint.
- Forward engineering: a model with a cycle emits both tables then one `ALTER TABLE`; the SQL applies cleanly on
  an empty database (integration test).
- Model file: save, reset the layout, load → positions restored.
- Dictionary: a table with a comment, an index and an FK appears with all three.
- Sweep covers the sixteen new ids.

## Done when

On all three engines a trigger can be listed, created and dropped from the app; a Postgres sequence can be
edited; a SQLite table's column type can be changed without losing its indexes; and a saved diagram reopens
where it was left.

## Ceilings

- **No SQL parser.** Validation is a prefix-and-name check; the server is the authority.
- **MySQL "edit" is drop-and-create**, reported as such.
- **Range partitions only**, date-typed. No list, hash, default partition, or `REORGANIZE`.
- **PNG may not land** if no rasteriser exists; SVG is the commitment.
- **The rebuild is SQLite-only** — the other engines alter in place and need none of it.
