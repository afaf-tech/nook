# Phase 3 — Reading data

**Arc A · closes `grid.blobEditor` · depends on phase 1**

## The problem, measured

The grid is Nook's strongest area on paper (89% of the Navicat Data area) and the weakest in practice,
because everything it can do, it does to **one cell of one row at a time**:

| Limitation | Evidence |
|---|---|
| Selection is a single cell | `table.rs:53` `pub selected: Option<(usize, usize)>` |
| No way to hide or reorder columns | no `hide_column`, `column_visibility` or `hidden_columns` anywhere (`grep` returns nothing) |
| A JSON or BLOB cell is a one-line string in a 28 px row | `Value::Json(String)`, `Value::Bytes(Vec<u8>)` (`core/src/value.rs:14`) |
| A foreign key is a number with no way to follow it | `grep -n "foreign" crates/ui/src/grid.rs` → nothing |
| Copy is per-cell | `grid.cellValue`, `ctx.copy_text` at `grid.rs:673` (pending SQL only) |
| No Excel-pasteable copy | `to_csv` / `to_markdown` / `to_sql_inserts` exist (`model/src/lib.rs:404–453`), no TSV |

TablePlus's answer to the third and fourth is Quick Look plus a right sidebar; Beekeeper's is a JSON
sidebar that nests FKs. Both matter more than any new dialog, because reading a row is the most common
thing anyone does with a database.

## Scope

| Command | What it does | Params |
|---|---|---|
| `grid.selection` | **new** — read/write the selection as a rectangle | `{"anchor":[0,0],"cursor":[5,3]}` |
| `grid.copyTsv` | **new** — the selection (or whole page) as TSV, header row optional | `{"header":true,"selection":true}` |
| `grid.hideColumns` | **new** — hide/show/reorder columns for a table, persisted per `conn + table` | `{"table":"users","hidden":["password_hash"],"order":["id","email"]}` |
| `grid.quickLook` | **new** — one cell, pretty-printed: JSON re-indented, BLOB as hex + a printable-text preview, long text wrapped | `{"row":3,"column":"payload"}` |
| `grid.rowJson` | **new** — a row as nested JSON, with each foreign key expanded one level | `{"row":3,"expand":["customer_id"]}` |
| `grid.followForeignKey` | **new** — from a cell holding an FK value, the referenced table and row, and the SQL to open it | `{"row":3,"column":"customer_id"}` |
| `grid.blobEditor` | **closes the Navicat id** — text and hex editing of `bytea`/`blob`, staged like any other edit | `{"row":3,"column":"payload","hex":"00ff10"}` |
| `grid.format` | **new, stretch** — Query Magics' cheap half: per-column display hints from the result's column aliases (`__format__money`, `__format__check`, `__format__link`) | `{"hints":["amount=__format__money"]}` |

## Design

### Selection is a rectangle, and the grid stops pretending otherwise

`TableState.selected: Option<(usize, usize)>` becomes:

```rust
pub struct Selection { pub anchor: (usize, usize), pub cursor: (usize, usize) }
```

`anchor` is where the drag or shift-click started, `cursor` is the live corner; the rectangle is derived.
Every existing call site that wants "the selected cell" reads `selection.cursor`. Shift+click, shift+arrow
and drag all set `cursor`; a plain click collapses both to the same cell. Painting: the existing
`tokens.selection` fill covers the rectangle, with the cursor cell drawn on top in `tokens.selection_strong`
(a token that must be added to `theme.rs` and verified at 4.5:1 like every other).

`grid.selection` exists so the rectangle is *scriptable* — a test and the control channel can assert what
the pointer did without synthesising mouse events.

### Copy is TSV by default, and it is one function

`Value::display()` already exists and is what the grid paints. TSV is
`rows.map(|r| r.map(display).join('\t')).join('\n')` — no quoting, because TSV's whole point is that
spreadsheets parse it and a quoted cell would arrive with quotes. Newlines inside a cell are the one real
hazard: they are replaced with `\n` (literal backslash-n) and a `"…"` wrapper is added only for that cell,
which is what Excel itself does on paste-back.

Add `ResultSet::to_tsv(&self, selection: Option<Rect>) -> String` in `crates/model/src/lib.rs` beside
`to_csv`, so the engine and the UI share one implementation — the conventions file is explicit that a
second version of the same job is a bug.

### Quick Look is a view, not a dialog

TablePlus opens a popup; Nook already has a JSON view in the results register (`register.rs::View::Json`)
and a form band in the grid. Quick Look therefore becomes a **cell inspector band** that reuses the
`dialog_shell` chrome: title is `table.column`, body is monospace text or hex, footer is "Copy" and
"Edit…" (which, for a BLOB, opens the blob editor). `grid.quickLook` returns the *text* the band paints,
so the engine owns the formatting rules and the UI owns the pixels — the same split as `grid.formView`.

Pretty-printing JSON without a crate: `serde_json::from_str::<serde_json::Value>()` then
`to_string_pretty`, which is already a dependency. Invalid JSON stays verbatim with a one-line note saying
so — never an error, because a `json` column with bad text is exactly when you need to look at it.

Hex: 16 bytes per line, offset column, ASCII gutter. ~20 lines of code, no crate.

### The JSON sidebar expands one level, on demand

`grid.rowJson` builds `{"column": value}` for the row, then for each requested FK column replaces the
scalar with `{"__ref__": "public.customers", "id": 42, "row": {...}}` by running one `SELECT * … WHERE pk = ?`
per expansion. Two rules:

1. **One level only.** A self-referencing FK would otherwise recurse forever; the JSON shows the nested row
   *without* its own expansions, and the UI offers a "follow" chevron instead.
2. **A failed expansion is data, not an error:** `{"__error__": "no row with id 42"}` inside the document, so
   a dangling FK does not blank the panel.

The UI renders it as a collapsible tree. `egui::CollapsingHeader` over a `serde_json::Value` walk is enough;
no tree widget needed.

### Foreign keys are clickable because the catalog already knows them

`Catalog::find_table(...).foreign_keys` (used by `schema.foreignKeys`) has the target table and columns.
`grid.followForeignKey` resolves the value, finds the target's primary key, and returns
`{"conn","schema","table","where":"id = 42"}`. The UI turns the cell into a link (underline in
`tokens.accent`) and opens a Data tab filtered to that row via `grid.filter`. If the target table has no
primary key, the command returns the row's `WHERE` from every column instead and says so — never silently
does nothing.

### Blob editing is a staged edit like any other

`Value::Bytes` is already a `Value`, and the pending-edit path (`plan_edits`, `grid.apply`) already handles
values. So the blob editor is: a band with a hex grid and a text pane, saving into the same
`PendingEdit` the inline editor would produce. The engine side is small: `grid.blobEditor` validates the
hex, decodes it, and calls the same `edit_cell` the grid uses. No new write path — that is the whole point
of `plan_edits` existing.

Refuse to open the editor for a BLOB over `max_blob_bytes` (default 1 MiB, a setting) and say the size;
loading a 2 GB `bytea` into a `Vec<u8>` to paint it is how a database client eats all the RAM.

### Query Magics: only the cheap half

Beekeeper's alias-driven formatting (`SELECT __format__stars(rating)`) is genuinely nice and genuinely
hard to justify as a product feature here. This phase ships **display hints** in the grid: a column whose
name carries `__format__money` is right-aligned with two decimals, `__format__check` paints a tick or a
cross, `__format__link` is clickable and opens the URL with the OS handler. No `enums.json`, no images, no
nested jumps. Marked stretch so it can be dropped without breaking the phase.

## Files

| File | Work |
|---|---|
| `crates/model/src/lib.rs` | `ResultSet::to_tsv`, `Selection`/`Rect` if it belongs here rather than in the UI |
| `crates/ui/src/table.rs` | rectangle selection, drag, shift-click, TSV copy, the cursor-cell token |
| `crates/ui/src/grid.rs` | hide/show columns, quick-look band, JSON panel, FK links, blob band |
| `crates/ui/src/theme.rs` | `selection_strong`, `accent` if absent; verify ratios |
| `crates/engine/src/cmd/grid.rs` | the eight commands, `quickLook` formatting, hex encode/decode, FK resolution |
| `crates/engine/src/store.rs` | hidden columns and column order, keyed by `conn + table` |
| `crates/engine/src/lib.rs` | `max_blob_bytes` setting |
| `crates/ui/src/control.rs` | `ui.selectRange` so a screenshot can prove the rectangle |

## Steps

1. `to_tsv` + `grid.copyTsv` with a unit test over a result set containing a newline, a tab and a NULL.
2. Rectangle selection in `table.rs`; `grid.selection`; `ui.selectRange`; screenshot.
3. `grid.hideColumns` persisted in the store; the UI's column menu.
4. `grid.quickLook` + the band (JSON and hex paths).
5. `grid.rowJson` + the panel, one-level expansion, the error-as-data rule.
6. `grid.followForeignKey` + the link.
7. `grid.blobEditor` with the size ceiling.
8. Stretch: display hints.

## Tests

- `to_tsv`: NULL is an empty cell, an embedded tab and newline are escaped, a header row is optional.
- Selection: shift-click from `(1,1)` to `(4,2)` is a 4×2 rectangle; a plain click collapses it.
- `grid.hideColumns`: hidden columns survive a reopen (store), and `grid.fetchPage` still returns them — hiding
  is a view concern and must not change the query.
- `quickLook`: valid JSON is re-indented; invalid JSON is verbatim with the note; 32 bytes render as two hex
  lines with an ASCII gutter.
- `rowJson`: a self-referencing FK stops after one level; a dangling FK produces `__error__` and does not fail.
- `followForeignKey`: target has a PK → `WHERE pk = …`; target has no PK → all columns, and the note.
- `blobEditor`: round-trips `00ff10`; refuses 1 MiB + 1 byte with the size in the message.
- Sweep: the new ids survive junk parameters.

## Done when

Dragging over cells selects a rectangle, `Cmd+C` pastes into a spreadsheet as a real table, `Cmd+P`-style
navigation to a referenced row works from a click, and a `jsonb` cell opens readable.

## Ceilings

- **TSV only, no clipboard HTML/RTF.** Excel and Sheets both parse TSV; the rich formats are a per-platform
  rabbit hole.
- **Hex editor has no diff.** It replaces the value; the pending list is the review.
- **One-level FK expansion.** A "follow" chevron is the path to more, deliberately not built here.
- **Display hints only.** No `enums.json`, no images, no cell-level action registry.
