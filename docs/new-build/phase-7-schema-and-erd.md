# Phase 7 — Schema depth and the diagram

**Arc C · closes `schema.tableRebuild`, `diagram.reverseEngineer`, `diagram.forwardEngineer`, `diagram.manualLayout`, `diagram.modelFile`, `tools.dataDictionary` · depends on phase 3**

> **Rewritten after rebasing onto `a66bcd2`.** The first draft opened this phase with the trigger, routine and
> sequence editors as its foundation. `origin/main` shipped them while the plan was being written (`014a124` —
> `schema.triggerEditor` / `functionEditor` / `sequenceEditor` with `action` list/create/drop, plus PostgreSQL and
> MySQL catalog introspection for all three), together with the SQLite rebuild machinery the draft had listed as
> missing (`4534c21`, `88c9979`, `476f7e9`, `ffbad6d`, `a66bcd2`, and `schema.setPrimaryKey`). Those three ids
> are **closed**, so this phase is scoped against what is genuinely left.

## The problem, measured

| Fact | Evidence |
|---|---|
| Triggers, routines and sequences are editable on all three engines | `cmd/schema.rs:59–67`; the catalog notes now read "list/create/drop via `action`" |
| A SQLite table can be rebuilt, transactionally | `Driver::rebuild_table` (`drivers/src/lib.rs:112`), `SqliteDriver::rebuild_table_plan` (`drivers/src/sqlite.rs:460`) |
| …but only one command reaches it | `grep -n "rebuild_table" crates/engine/src/cmd/schema.rs` → `set_primary_key`. `schema.tableRebuild` is still catalogued "SQLite DDL that needs it says so instead" |
| A rebuild that takes a column list loses data by omission | `rebuild_table_plan` rebuilds from `new_columns`. The shipped path is safe only because `set_primary_key` clones the table first; a general command invites a caller to rebuild that list by hand, and the first one that forgets a column drops its data |
| No partitions anywhere | `grep -rn "partition" crates/drivers/src crates/engine/src/cmd/schema.rs` → nothing. Postgres lists a partitioned table (`pg.rs`, `relkind IN ('r','p','v','m','f')`) but not its children |
| The diagram is one-way and un-editable | `cmd/diagram.rs` has three commands (`erDiagram`, `exportSvg`, `tableCard`); `layout_diagram` is deterministic, no positions are stored, there is no model file and no forward engineering |
| No data dictionary | `tools.dataDictionary` catalogued, unimplemented |
| MySQL routine DDL is empty by choice | `docs/gaps.md` §4: "rather than one `SHOW CREATE` round trip per object" — an inherited ceiling, not a bug |

So the remaining gap is three things: **the rebuild as a first-class, safe command**, **partitions**, and **the
diagram becoming a document you can edit, save and forward-engineer**.

## Scope

| Command | What it does | Params |
|---|---|---|
| `schema.tableRebuild` | **closes the Navicat id** — SQLite's rebuild driven by a *diff*, not a column list, so nothing is lost by omission | `{"table":"users","changes":{"rename":{"email":"mail"},"retype":{"total":"TEXT"}},"confirm":true}` |
| `schema.partitions` | **new** — a partitioned table's partitions, with bounds and row estimates | `{"table":"events"}` |
| `schema.createPartition` / `schema.dropPartition` | **new** — range-by-date only: Postgres `CREATE TABLE … PARTITION OF … FOR VALUES FROM … TO …`, MySQL `ALTER TABLE … ADD PARTITION` | `{"parent":"events","name":"events_2026_11","from":"2026-11-01","to":"2026-12-01"}` |
| `diagram.reverseEngineer` | **closes the Navicat id** — the live catalog as a model document (tables, columns, keys, positions) | `{"schema":"public"}` |
| `diagram.forwardEngineer` | **closes the Navicat id** — a model document to DDL, ordered by dependency | `{"model":{…}}` |
| `diagram.manualLayout` | **closes the Navicat id** — store box positions per `conn + schema` | `{"positions":{"users":[40,80]},"reset":false}` |
| `diagram.modelFile` | **closes the Navicat id** — save/load the model, positions included, as JSON | `{"path":"~/models/shop.json","action":"save"}` |
| `diagram.exportPng` | **new, conditional** — PNG beside the SVG, *only if* a rasteriser already exists (step 0 decides) | `{"path":"~/diagrams/schema.png"}` |
| `tools.dataDictionary` | **closes the Navicat id** — every table, column, type, key, default and comment as Markdown or HTML | `{"schema":"public","format":"markdown","path":"~/dict.md"}` |

Already shipped on `origin/main`, listed here so nobody re-plans them: `schema.triggerEditor`,
`schema.functionEditor`, `schema.sequenceEditor` (list/create/drop, three engines, catalog introspection) and
`schema.setPrimaryKey` (the first caller of the rebuild).

## Design

### The rebuild takes a diff, because a full column list is how data gets lost

The shipped `rebuild_table(schema, table, new_columns)` is correct and already transactional, and
`set_primary_key` uses it safely: it clones the table, adds the key, and passes the whole column list. A general
`schema.tableRebuild` that also took "the new columns" would invite every caller to reconstruct that list by
hand, and the first one that forgets a column silently drops its data.

So the command takes **changes**, and builds the new column list itself from the live catalog:

```json
{"changes": {"rename":   {"email": "mail"},
             "retype":   {"total": "TEXT"},
             "nullable": {"note": true},
             "default":  {"status": "'new'"},
             "drop":     ["legacy_id"],
             "add":      [{"name": "tag", "type": "TEXT"}]}}
```

Every key is optional. The command reads the table from the catalog, applies the changes to a clone, and calls
the existing `rebuild_table` with the resulting columns — so there is exactly one rebuild implementation and the
command cannot invent a second one. It then **verifies before it commits**: every index, trigger and foreign key
present on the original table must appear on the rebuilt one, compared by name. The shipped plan already carries
triggers, session pragmas and foreign keys across (`88c9979`, `4534c21`); this check is the proof, not a second
implementation.

Three refusals:

1. **Inside an open transaction** → refuse. `set_primary_key` already does this (`cmd/schema.rs:386–391`); reuse
   the same message, because the pragma cannot change inside one.
2. **`foreign_key_check` returns rows** → roll back and list them.
3. **An index, trigger or FK the plan cannot replay** → refuse and name it. The ones it cannot replay are exactly
   the ones whose loss would be invisible.

`schema.changeColumnType` on SQLite should then route through this command instead of refusing — the one
behaviour change that makes the command earn its place rather than being a second way to do what
`schema.setPrimaryKey` does.

### Partitions, narrowly

Read: Postgres `pg_inherits` joined to `pg_class`, with `pg_get_expr(c.relpartbound, c.oid)` for the bounds;
MySQL `information_schema.PARTITIONS` (`PARTITION_NAME`, `PARTITION_DESCRIPTION`, `TABLE_ROWS`). Both return
`{parent, name, from, to, rows}`, painted as children of the parent table in the tree.

Write: **range partitions by date only**, which is the case anyone actually uses —
`CREATE TABLE events_2026_11 PARTITION OF events FOR VALUES FROM ('2026-11-01') TO ('2026-12-01')`. List, hash,
default partitions and MySQL's `REORGANIZE PARTITION` are refused with a sentence pointing at the query editor.
Drop is `DROP TABLE` on the partition (Postgres) or `ALTER TABLE … DROP PARTITION` (MySQL), behind `confirm`.

### The diagram gets positions, a model file and a second export

`nook_model::layout_diagram` is deterministic and stays the default. Add:

- `Diagram.boxes: Vec<(String, [f32; 2])>` — the layout's own output, so a caller can read positions without
  re-running the layout.
- `diagram.manualLayout` stores overrides in the store keyed by `conn + schema`; `reset: true` clears them.
- `diagram.modelFile` writes `{"version":1,"conn":"local","schema":"public","boxes":[…],"tables":[…]}`. The
  tables are in the file so it is readable without a live connection — that is what makes it a *document* rather
  than a bookmark.
- `diagram.forwardEngineer` takes that document and emits `ddl::create_table` per table in dependency order
  (topological sort over `foreign_keys`), with cycles broken by emitting the FK as a separate
  `ALTER TABLE … ADD CONSTRAINT` after every table exists — the only correct answer for a cycle.

**PNG is conditional, and step 0 is `grep -rn "png" crates apps`.** The `--shot` flag renders a frame, so a
rasteriser may exist; if it is `eframe`-only, rendering a diagram offscreen needs the `egui` painter into a
`ColorImage` plus a PNG encoder. If neither is already there, the item is dropped and recorded as a ceiling — SVG
is the commitment, and adding an image-encoding dependency for a second export format is not worth it.

### Data dictionary

`schema.ddl` already projects the catalog into DDL; the dictionary is a different projection of the same data:
table → columns (name, type, nullability, default, key, comment), indexes, foreign keys, row estimate. Markdown
for a wiki, self-contained HTML for a share. One pass over `Catalog`, no new queries.

## Files

| File | Work |
|---|---|
| `crates/engine/src/cmd/schema.rs` | `schema.tableRebuild` (the diff), `schema.partitions`, `createPartition`, `dropPartition`; route SQLite `changeColumnType` through the rebuild |
| `crates/drivers/src/{pg,mysql}.rs` | partition read, and MySQL's `ADD`/`DROP PARTITION` DDL |
| `crates/model/src/ddl.rs` | `create_partition`, `drop_partition`, dependency-ordered forward engineering |
| `crates/model/src/diagram.rs` | `boxes`, overrides, the model document (serde) |
| `crates/engine/src/cmd/diagram.rs` | `reverseEngineer`, `forwardEngineer`, `manualLayout`, `modelFile`, `exportPng` (if step 0 allows) |
| `crates/engine/src/cmd/tools.rs` | `tools.dataDictionary` |
| `crates/engine/src/store.rs` | diagram positions |
| `crates/ui/src/views.rs` | partitions under the parent table in the tree; the diagram canvas with drag |
| `docs/architecture.md` | the rebuild's diff contract, the partition ceiling, the PNG decision |

## Steps

0. `grep -rn "png" crates apps` and `grep -rn "ColorImage" crates` — settle the PNG item before writing anything.
1. `schema.partitions` (read-only, both engines) and the tree children.
2. `schema.tableRebuild` with the diff, the three refusals, and the post-rebuild index/trigger/FK verification;
   route SQLite's `changeColumnType` through it.
3. `createPartition` / `dropPartition`, range only, `confirm` on drop.
4. `diagram.reverseEngineer` (the catalog as a document) and `boxes` in the model.
5. `manualLayout` + drag on the canvas + `modelFile`.
6. `forwardEngineer`, including the cycle path.
7. `tools.dataDictionary`.
8. `exportPng` if step 0 said yes; otherwise record the ceiling.
9. Regenerate parity; six ids leave the gap list.

## Tests

- Rebuild by diff: rename a column and the row count, index and trigger survive; retype a column and the values
  round-trip; a change that would orphan a reference is rolled back with the offending rows listed; a rebuild
  inside an open transaction is refused with the shipped message.
- Rebuild verification: a table with an index the plan cannot replay is refused, and the index is named.
- Partitions: create a range partition, insert through the parent, and the row is in the child (assert the
  child's count); a list partition is refused with the query-editor hint; drop needs `confirm`.
- Forward engineering: a model with a cycle emits both tables then one `ALTER TABLE`; the script applies cleanly
  on an empty database (integration test).
- Model file: save, reset the layout, load → positions restored.
- Dictionary: a table with a comment, an index and an FK appears with all three.
- Sweep covers the eight new ids.

## Done when

A SQLite column can be renamed and retyped from the app without losing its indexes or triggers, a Postgres range
partition can be created from the app, and a saved diagram reopens where it was left and forward-engineers into
DDL.

## Ceilings

- **Range partitions only**, date-typed. No list, hash, default partition, or `REORGANIZE`.
- **The rebuild is SQLite-only** — the other engines alter in place and need none of it.
- **PNG may not land**; SVG is the commitment, and the decision is recorded in `docs/architecture.md`.
- **The diagram has no physics**; positions are stored, not simulated.
- **MySQL routine DDL stays empty** (inherited from `docs/gaps.md` §4) — the dictionary shows a MySQL routine's
  name and kind, not its body.
