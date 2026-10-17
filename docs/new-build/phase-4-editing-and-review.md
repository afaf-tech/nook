# Phase 4 — Editing and review

**Arc B · closes `grid.rawMode` · depends on phase 3**

## The problem, measured

Nook already has the thing TablePlus calls "code review" and Beekeeper calls "stage changes": edits
accumulate in `grid.pending`, `grid.copyAsSql` shows the statements, `grid.apply` runs them in one
transaction that rolls back at the first failure. That part is genuinely good and older than this plan.

What is missing is everything around it:

| Gap | Evidence |
|---|---|
| Filtering is one raw `WHERE` typed by hand | `grid.filter` takes `{"where":"status = 'live'"}` (`cmd/grid.rs:27`) — no builder, no multiple conditions, no SQL preview before it runs |
| Editing many rows means editing them one at a time | selection is one cell (`table.rs:53`), so there is no "set these 40 rows' status" |
| A `SELECT`'s results cannot be edited | the register is read-only by design (`register.rs` `READ_ONLY`) |
| "Safe mode" is a boolean | `settings.confirm_destructive` gates the `confirm: true` requirement; there is no second check at the moment of commit |
| The SQL of a pending change cannot be edited | `grid.rawMode` is catalogued as not implemented |

Beekeeper lets you edit a query's results and write the change back; TablePlus puts a filter builder in
front of the grid and asks for your password before a destructive commit. Both are about the same thing:
**the user should be able to change many rows deliberately, and see exactly what will run.**

## Scope

| Command | What it does | Params |
|---|---|---|
| `grid.filters` | **new** — structured filters as a list; compiles to the same `WHERE` `grid.filter` takes | `{"filters":[{"column":"status","op":"=","value":"live"},{"column":"total","op":">","value":"100"}]}` |
| `grid.filterPreview` | **new** — the `WHERE` (and the `COUNT(*)`) the filters would produce, without changing the grid | same as above |
| `grid.bulkEdit` | **new** — set one column across the selected rows, staged as pending edits | `{"column":"status","value":"archived","rows":[3,4,9]}` |
| `grid.rawMode` | **closes the Navicat id** — the exact SQL of a pending change, editable, with the edit re-derived from it | `{"index":0,"sql":"UPDATE users SET status = 'x' WHERE id = 3"}` |
| `grid.editQueryResult` | **new** — make a `SELECT`'s result editable by resolving its source table and primary key from the statement | `{"statement":0}` |
| `editor.safeMode` | **new** — require a re-authentication (keychain read or a typed confirmation) before `grid.apply` or any destructive command when enabled | `{"enabled":true}` |

## Design

### Filters are a builder over the string the grid already understands

The engine's filter path is `grid.filter` → a raw `WHERE` appended to the page query. Do not change that.
Add a compiler in front of it:

```rust
fn compile(filters: &[Filter]) -> Result<String>   // "status = 'live' AND total > 100"
```

Each value goes through `nook_core::sql_literal`, each column through `quote_ident` — the conventions file
says input-derived values are hostile, and a filter value typed into a box is exactly that. Operators are a
closed list (`=`, `!=`, `<`, `<=`, `>`, `>=`, `like`, `ilike`, `in`, `is null`, `is not null`, `between`),
so no operator string ever reaches the server unvalidated.

`grid.filters` stores the structured list on the grid tab *and* sets the compiled `WHERE`, so paging, sorting
and counting keep working unchanged and the UI can show chips. `grid.filter` stays as the escape hatch for
someone who wants to type SQL.

The UI is a row of chips above the grid (`+ Filter` opens a column/operator/value popover), with a "View SQL"
toggle that shows `grid.filterPreview`'s output. That preview is the TablePlus "SQL preview" feature and it is
also the honest way to teach the user what the grid is doing.

### Bulk edit reuses the pending path

`grid.bulkEdit` is a loop that builds the same `PendingEdit` the inline editor builds — one per row — and
then does nothing else. `grid.apply` already knows how to run them in one transaction. The only new logic is:

- the column must exist and be writable (not generated, not a PK when the value would collide — let the
  server say so, but refuse obviously-wrong writes like setting a PK to the same value across rows);
- the row count is capped by `max_rows` and the command says how many were staged;
- a value of `null` is `Value::Null`, not the string `"null"`.

### Editing a query's results, honestly

This is the feature most likely to lose someone's data if it is done sloppily, so it is done narrowly:

1. Parse the statement (`sqltext::statement_at` already splits statements). Accept it only if it is a single
   `SELECT … FROM <one table>` with no `GROUP BY`, no aggregate in the projection, no `UNION`, no `DISTINCT`.
   Anything else is refused with the reason.
2. Resolve `<one table>` against the live catalog. It must have a primary key.
3. The projection must include every PK column (or be `*`). If a PK is missing, refuse and name it — without
   the key there is no row to update, and guessing is how a client corrupts data.
4. From then on the tab behaves exactly like a Data tab: same pending list, same `plan_edits`, same
   one-transaction apply. `grid.editQueryResult` returns the resolved table, the key columns and a
   `read_only: false` flag; the UI flips the register's `editable` and its footer note from
   "read-only result" to the table's key.

Refusals are the feature. A client that silently makes a `GROUP BY` result editable is a client that will
one day update 400 rows to the same value.

### Raw mode is a review surface, not a query editor

`grid.rawMode` returns the SQL a pending edit will run. Setting it re-derives the edit: parse the `UPDATE …
SET … WHERE …` (the only shape the planner emits), match the `WHERE` back to a row in the page by primary
key, and replace that edit's value. If the SQL cannot be mapped back to exactly one pending edit, refuse —
do not append an arbitrary statement to the pending list. Someone who wants arbitrary SQL has the query
editor; conflating the two is how a "review" pane becomes a footgun.

### Safe mode is a gate at the moment of commit

`confirm_destructive` already refuses destructive commands without `confirm: true`. Safe mode adds a
*second* check that cannot be passed by a caller who merely sets the flag:

- On enable, `editor.safeMode` stores a random 32-byte token in the keychain (the existing `security`
  seam, `profiles.rs:182`) under `nook.safemode.<conn id>`.
- `grid.apply` and every `confirm`-gated command, when safe mode is on, require `{"safe_token": "…"}` in the
  params. The UI gets the token by prompting for the connection's password and verifying it with a real
  `conn.test` — or, for a keychain-less connection, by typing the connection name.
- A caller that does not have the token is refused with `Error::config("safe mode: confirm the connection
  password")`.

This is deliberately *not* a password re-entry that the app compares itself: it is a live connection test,
so a rotated password fails and an unattended script cannot bypass the gate by reading a file. Record the
ceiling: an attacker with the keychain and the shell can still get through, because they have the
credentials anyway.

## Files

| File | Work |
|---|---|
| `crates/engine/src/cmd/grid.rs` | `filters`, `filterPreview`, `bulkEdit`, `rawMode`, `editQueryResult`, the safe-mode check in `apply` |
| `crates/engine/src/sqltext.rs` | `single_select_source(sql) -> Option<Source>` — the parse the editability rule needs |
| `crates/engine/src/cmd/sql.rs` | `editor.safeMode` (or `app.setSetting`'s `safe_mode` key) |
| `crates/engine/src/lib.rs` | `GridState.filters: Vec<Filter>`, `SqlState.editable: Option<EditableSource>` |
| `crates/engine/src/profiles.rs` | keychain token helpers beside `db_password` |
| `crates/ui/src/grid.rs` | filter chips + popover + SQL preview, bulk-edit band, raw-mode pane |
| `crates/ui/src/register.rs` | editable results: footer note, dirty markers, pending band |
| `crates/ui/src/editor.rs` | the safe-mode prompt |
| `docs/architecture.md` | the editability rules and the safe-mode ceiling |

## Steps

1. `compile(filters)` with a table test per operator, including `in` and `between` and NULL handling.
2. `grid.filters` / `grid.filterPreview` on the grid tab; chips in the UI.
3. `grid.bulkEdit` over the phase-3 rectangle; staged edits appear in the pending list.
4. `single_select_source` + `grid.editQueryResult`; the register flips to editable; the refusals.
5. `grid.rawMode` with the map-back-or-refuse rule.
6. Safe mode: token in the keychain, the gate in `apply`, the prompt.
7. Regenerate parity; `grid.rawMode` leaves the "not implemented" list.

## Tests

- Filters: every operator compiles to the expected `WHERE`; a value containing `'` is escaped; an unknown
  operator is refused; `is null` takes no value.
- Bulk edit: 3 rows → 3 pending edits; a NULL value is `Value::Null`; setting a PK across rows is refused.
- `single_select_source`: accepts `select * from users`; refuses `group by`, `distinct`, a join, two tables,
  an aggregate, a statement with no `from`.
- Editability: a `SELECT` missing the PK is refused and the message names the missing column.
- Raw mode: a matching `UPDATE` replaces the edit's value; an `UPDATE` with a `WHERE` matching no pending row
  is refused, and the pending list is unchanged.
- Safe mode: with it on, `grid.apply` without a token is refused; with the right token it applies; with safe
  mode off, behaviour is exactly today's.
- Sweep covers the six new ids.

## Done when

A user can filter a table with three chips, see the SQL that filter produces, set one column across 40
selected rows, review each statement, and commit — and an operator who typed `select status, count(*) …
group by status` cannot accidentally make that result editable.

## Ceilings

- **Filter groups are flat.** No nested AND/OR tree. `grid.filter`'s raw `WHERE` is the escape hatch, and a
  tree needs a UI nobody has asked for yet.
- **Raw mode is one statement shape.** It is a review surface; arbitrary SQL belongs in the editor.
- **Safe mode is a token, not a session re-auth.** It stops a stray script and a slip of the mouse, not a
  determined local attacker. Say so in the setting's own help text.
- **No undo.** Discard is `grid.revert`; an applied transaction is a real transaction.
