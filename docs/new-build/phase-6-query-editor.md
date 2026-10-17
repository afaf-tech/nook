# Phase 6 — The query editor

**Arc B · closes `sql.parameterised` · depends on phase 1**

## The problem, measured

The editor runs scripts, statements and selections, formats, completes from the live catalog, explains and
keeps history. What it cannot do is the set of things a person typing SQL for an hour actually wants:

| Gap | Evidence |
|---|---|
| No query parameters | `sql.parameterised` is catalogued "not implemented; literals are escaped on the client" (`catalog.rs:431`) |
| No find & replace | `grep -n "find\|replace" crates/ui/src/editor.rs` matches completion plumbing only — nothing that searches the text |
| One editor per tab, no split | `Tab.sql: SqlState` holds one `text`; no split/pane concept |
| No zen mode | `grep zen crates/ui` → an icon name only |
| No multiple carets | `editor.rs` uses egui's single-cursor `TextEdit` |
| History is a flat list | `sql.history` returns the last N; no per-connection or per-tab scoping, no "reuse this one" |

TablePlus's split panes and parameterised queries, Beekeeper's zen mode and find & replace. Parameters are
the one with teeth: today every value is interpolated client-side through `sql_literal`, which is correct
but means a query cannot be prepared, reused, or handed to a colleague without editing literals.

## Scope

| Command | What it does | Params |
|---|---|---|
| `sql.parameters` | **new** — find the parameter placeholders in a script and their current values | `{"sql":"select * from u where email = :email"}` → `{"style":"colon","params":[{"name":"email","value":null,"count":1}]}` |
| `sql.setParameter` | **new** — bind a value (kept per tab, cleared on request) | `{"name":"email","value":"a@b.c"}` |
| `sql.run` (extend) | when parameters are present and bound, **send them as a prepared statement**; unbound → refuse with the names | `{"sql":"… :email …","params":{"email":"a@b.c"}}` |
| `sql.parameterised` | **closes the Navicat id** — the capability, reported by `sql.parameters` + the prepared run | — |
| `sql.findReplace` | **new** — find, replace one, replace all, case-sensitive and whole-word toggles | `{"find":"users","replace":"accounts","all":true,"case":false,"word":true}` |
| `sql.zen` | **new** — editor full-bleed, panes hidden | `{"on":true}` |
| `sql.splitPane` | **new** — the tab's editor beside a second editor, sharing the connection | `{"tab":0,"side":"right"}` |
| `sql.multiCaret` | **new, stretch** — add a caret at the next occurrence of the selection, type into all | `{"add":"next"}` |
| `sql.history` (extend) | filter by connection, and `{"reuse":0}` to put an entry back in the editor | `{"conn":"local","limit":50}` |

## Design

### Parameters: two styles, one parser, real prepared statements

TablePlus offers four regex styles (`:name`, `%name%`, `?`, `${name}`) and calls them query params. Two of
them are worth having, and the reason is that they mean different things:

- **Named** `:name` (and `${name}` as an alias) — a value the user types once and reuses.
- **Positional** `?` — a value per occurrence, which is what a prepared statement actually is.

`%name%` is rejected: it collides with SQL's own `LIKE '%x%'` pattern and would silently eat a wildcard. Say
so in the command's help, once.

Detection happens on the token stream that `sqltext::tokenize` already produces, not on a regex — a `:name`
inside a string literal or a comment is not a parameter, and a regex cannot tell. `sql.parameters` returns
the style, the names in order of appearance, how many times each occurs, and the current bindings.

**Binding is per tab** (`SqlState.params: BTreeMap<String, Option<String>>`), so switching tabs does not
leak one query's values into another. A binding is `Option<String>` because "explicitly NULL" and "not set"
are different, and the difference is a query that returns everything versus nothing.

**The run path.** When parameters are present:

| Driver | What happens |
|---|---|
| PostgreSQL | the current driver uses the simple query protocol (`docs/gaps.md`, "Known quality ceilings"). Prepared statements need the extended protocol — `postgres` crate's `prepare`/`query` — so this phase adds `Driver::run_prepared(sql, &[Value], max) -> ResultSet` and implements it for Postgres and MySQL; SQLite gets `rusqlite::Statement::bind`. |
| MySQL | `mysql` crate: `prep`/`exec`. |
| SQLite | `rusqlite`: `prepare` + `bind` + `raw_bind_parameter`. |

`Driver::run_prepared` gets a default implementation that returns `Error::Unsupported("this driver runs
prepared statements…")` — the same seam style the catalog already uses for unimplemented features. **No
client-side substitution as a fallback**: if a driver cannot prepare, the query is refused with the reason,
because a silent fallback to interpolation is exactly the ceiling this phase exists to remove.

An unbound parameter refuses the run and names the parameters — never sends `:email` to the server as a
syntax error, which is what happens today.

### Find & replace is text surgery, and it must not fight the caret

`sql.findReplace` operates on `tab.sql.text` and returns the new text plus the ranges it touched, so the UI
can scroll to and highlight each hit. Rules that matter:

- Empty `find` → refused (an empty pattern would insert `replace` between every character).
- Whole-word matching uses the same word-boundary test `editor.rs::replace_prefix` already uses for
  completion, so `user` does not match inside `users` when `word: true`.
- Replace-all builds the new string in one pass over the match list, right to left, so offsets stay valid.
- The caret and the scroll position are restored relative to the text, not to a byte offset — the editor's
  caret helper (`editor.rs::caret_in`) already maps between them.

The UI: `Cmd+F` opens a find bar at the top of the editor, `Cmd+Alt+F` adds the replace field, Enter walks
hits, `Esc` closes and restores focus.

### Zen mode is a layout flag, not a new screen

`UiState.zen: bool`. When on: the app bar, tree, inspector, tab strip and status bar stop painting and the
active tab's editor fills the viewport; `Esc` or `Cmd+Shift+Z` restores. Nothing else changes — the same
tab, the same text, the same keyboard shortcuts. One flag and one early return in `app.rs::update`, plus the
`ui.state` payload reporting it so a screenshot can prove it.

### Split panes share the connection, not the text

Two editors, one tab. `SqlState` gains:

```rust
pub split: Option<SplitState>,   // { text: String, ran_script: String, results: Vec<StatementRun> }
```

The left pane keeps the tab's own `text` and `results`; the right pane is `SplitState`. Both run against the
tab's connection, both have their own result register (the register already keys on tab index — it becomes
`(tab, pane)`), and each keeps its own pending run. `sql.splitPane` toggles it; `ui.setResultView` and
`ui.selectResult` gain a `pane` field defaulting to `"left"`, so the control channel stays honest.

Deliberately *not* built: dragging the divider to resize (fixed 50/50 with a `theme` token for the gap),
and splitting into more than two.

### History gets a connection filter and a reuse path

`sql.history` currently returns the last N from memory (now persisted, phase 1). Add `conn` filtering and a
`reuse` parameter that copies an entry's SQL back into the active tab — the thing every history panel is
for, and today requires retyping.

### Multiple carets: stretch, and honestly limited

egui's `TextEdit` is single-cursor. Real multi-caret needs either a custom text widget (a large, risky
change to the editor that this plan does not justify) or a narrow trick: track a list of extra carets,
apply each keystroke to every caret's range from last to first. Ship the narrow version for `Cmd+D`
(select-next-occurrence) and typing only, with a documented ceiling: no multi-caret for Enter, paste, or
undo. If the narrow version fights the widget, drop it — it is the one item in this phase that may not land.

## Files

| File | Work |
|---|---|
| `crates/engine/src/sqltext.rs` | `parameters(sql) -> Params`; the token-based scan |
| `crates/engine/src/cmd/sql.rs` | `sql.parameters`, `sql.setParameter`, parameter-aware `sql.run`, `sql.findReplace`, `sql.zen`, `sql.splitPane`, `sql.multiCaret`, history filters |
| `crates/engine/src/lib.rs` | `SqlState.params`, `SqlState.split`, `UiState`-adjacent flags |
| `crates/drivers/src/lib.rs` | `Driver::run_prepared` + default refusal |
| `crates/drivers/src/{pg,mysql,sqlite}.rs` | the three implementations |
| `crates/ui/src/editor.rs` | find bar, parameter chips above the editor, split rendering, zen |
| `crates/ui/src/register.rs` | per-pane registers |
| `crates/ui/src/app.rs` | `Cmd+F`, `Cmd+Shift+Z`, `Cmd+D`, zen early-return, split layout |
| `crates/ui/src/control.rs` | `pane` on the two existing methods, `ui.zen` |

## Steps

1. `parameters()` over the token stream, with a table of cases: in a string, in a comment, `::` cast (Postgres
   type cast is not a parameter — this is the classic bug), `?` inside a literal.
2. `Driver::run_prepared` for SQLite first (fastest to test), then Postgres, then MySQL; each with a test that
   a bound value never appears in the SQL text sent.
3. Parameter chips in the editor + `sql.setParameter`; a run with an unbound parameter refuses and names it.
4. `sql.findReplace` with the caret-preservation rules.
5. Zen mode.
6. Split panes + per-pane registers.
7. History filter and reuse.
8. Stretch: multi-caret.
9. Regenerate parity; `sql.parameterised` leaves the gap list.

## Tests

- `parameters`: `select * from u where e = :email and x::int = 1` → one parameter; `'a:name'` → none;
  `-- :x` → none; `?` twice → two positional slots.
- Prepared: a value containing `'` and a `;` runs and does not change the statement's shape; the driver records
  the SQL it prepared and the test asserts the placeholder, not the value, is in it.
- Unbound: refuses with the parameter's name; nothing reaches the driver.
- Find & replace: whole-word does not touch `users` when finding `user`; replace-all offsets are correct when
  a replacement is longer than the match; an empty find is refused.
- Split: two panes run different SQL on one connection and keep separate results; closing the split discards
  the right pane's results.
- Zen: `ui.state` reports it; no pane rects are painted (screenshot).
- Sweep covers the new ids.

## Done when

A query with `:email` runs as a real prepared statement on all three engines, `Cmd+F` replaces inside the
editor without moving the caret somewhere wrong, and `Cmd+Shift+Z` gives a full-bleed editor.

## Ceilings

- **Two panes, fixed split.** No drag, no three-way.
- **Named + positional parameters only.** `%name%` is refused with a reason.
- **No parameter history or saved parameter sets.** Bindings live and die with the tab.
- **Multi-caret is select-next-occurrence and typing only.** Droppable.
- **Prepared statements do not change the grid/import paths** — those still build client-side literals
  (`docs/gaps.md` ceiling), and this phase does not rewrite them.
