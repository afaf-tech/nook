# Phase 2 — Open Anything (`Cmd+P`)

**Arc A · closes `tools.serverSearch` · depends on phase 1**

## The problem, measured

Nook has a palette, and it searches the wrong thing. `dialogs.rs::palette` filters
`nook_engine::registry()` by id, label and group — every match is a *command*. TablePlus's `Cmd+P`
("Open Anything") and Beekeeper's global quick search both open an **object**: a table, a view, a
schema, a saved query. Nook's tree filter (`tree.rs:191`) finds objects, but only inside the sidebar,
only by name, and it does not open anything.

So the tool has two half-searches and no way to type a table name and land in it.

## Scope

| Command | What it does | Params |
|---|---|---|
| `quickOpen.search` | **new** — one query, ranked hits across every connected catalog plus saved queries and open tabs | `{"query":"cust","limit":40}` |
| `quickOpen.open` | **new** — open the hit: a table/view becomes a Data tab, a saved query becomes a Query tab with its SQL, an open tab becomes active | `{"kind":"table","conn":"local","schema":"main","name":"customers"}` |
| `tools.serverSearch` | **closes the Navicat id** — the same index, server-wide, with a `pattern` param, kept for parity and scripting | `{"pattern":"order"}` |

## Design

**One index, built from what is already in memory.** Every open `Conn` holds `catalog: Option<Catalog>`
(`cmd/mod.rs::open_conn` fills it). `Catalog` already answers `table_keys()`, `column_names()`,
`find_table()`, `schema_names()` (`crates/model/src/lib.rs:252–295`). The index is a `Vec<Entry>` built on
demand — no cache, no invalidation problem, and it costs one pass over catalogs that are already parsed:

```rust
struct Entry { kind: HitKind, conn: String, schema: Option<String>, name: String, haystack: String }
```

`haystack` is `schema.name` lowercased plus, for tables, the column names — so typing `email` finds the
table that has an `email` column, which is the thing both competitors do and Nook cannot.

**Ranking, not filtering.** Substring-only matching buries `customers` under `customer_audit_log` when you
type `customers`. Score, then sort, then truncate:

| Signal | Score |
|---|---|
| exact name match | 100 |
| name starts with the query | 60 |
| name contains the query | 40 |
| a word in the name starts with the query (`cust` → `customer_audit`) | 30 |
| a column name starts with the query | 15 |
| schema or connection name matches | 10 |

No fuzzy crate. The three cases above are a `to_ascii_lowercase` and two `starts_with`; a real fuzzy
matcher is the upgrade path, not the first version.

**Saved queries join the index.** After phase 1, snippets are rows. A saved query is a hit with
`kind: saved`, ranked by name, and opening it creates a Query tab with its SQL. This is the feature
Beekeeper calls "online query storage" minus the online.

**Open tabs join the index** at the lowest rank, so `Cmd+P` also becomes "jump to the tab I lost".

**UI.** The existing palette becomes one overlay with two modes, not two overlays:

- `Cmd+K` → commands (today's behaviour, unchanged).
- `Cmd+P` → objects, saved queries, tabs.
- A `>` prefix in the field switches to commands, `#` to tabs — the convention every editor already
  taught the user.

`dialogs.rs::palette` grows a `mode` in `UiState` and one `match` on it. The list rendering, keyboard
handling, Enter-runs-first and Esc all stay exactly as they are; only the source of `matches` changes.
`ui.quickOpen` is added to `control.rs::METHODS` with `{"mode":"objects","query":"cust"}` so the mode is
scriptable and screenshot-verifiable.

**Multi-connection.** Hits carry the connection id, and the list shows a right-aligned dim chip with the
connection name when more than one is open — otherwise the same table name in three databases is a
coin flip.

## Files

| File | Work |
|---|---|
| `crates/engine/src/cmd/tools.rs` (or a new `cmd/open.rs`) | `quickOpen.search`, `quickOpen.open`, `tools.serverSearch`; the index and the scoring |
| `crates/engine/src/cmd/mod.rs` | register the module |
| `crates/engine/src/lib.rs` | `UiState`-free helper `Session::catalog_index()` if it needs to borrow two things at once |
| `crates/ui/src/dialogs.rs` | palette modes, `>`/`#` prefixes, the connection chip |
| `crates/ui/src/app.rs` | `Cmd+P` keybinding beside the existing `Cmd+K` (`app.rs:2536`) |
| `crates/ui/src/control.rs` | `ui.quickOpen` |
| `docs/parity-checklist.md` | regenerate — `tools.serverSearch` moves out of "Not implemented" |

## Steps

1. The index and the scoring, as a pure function with a unit test over a hand-built catalog. No UI.
2. `quickOpen.search` + `tools.serverSearch` returning JSON; test ranking order explicitly (exact, prefix,
   contains, column, schema).
3. `quickOpen.open` for each kind; test that opening a table creates a Data tab with the right
   `TableRef`, and that a missing connection is an error naming the id.
4. Palette modes in `dialogs.rs`; `Cmd+P` in `app.rs`; the connection chip.
5. `ui.quickOpen` in the control channel, then drive the app and capture a PNG of the overlay open with a
   query typed.

## Tests

- Ranking: `customers` beats `customer_audit_log` for `customers`; `email` finds `users` via a column.
- Empty query returns nothing (not everything) — the list is a search result, not a browser.
- A hit in a closed connection never appears; a hit in an open one carries its `conn` id.
- `quickOpen.open` on a `saved` hit produces a Query tab whose `sql.text` equals the snippet's SQL.
- `tools.serverSearch` closes the catalog id: assert `nook_engine::parity()` no longer lists it.
- The junk-parameter sweep covers the two new ids.

## Done when

Typing `cust` after `Cmd+P` lists tables, the overlay opens the highlighted one on Enter, and
`tools.serverSearch` with `{"pattern":"cust"}` returns the same hits as JSON over the MCP control
channel (`nook-cli mcp`) — the CLI has no generic `run` verb, so MCP stdio is the scripted surface.

## Ceilings

- **No fuzzy matching.** `strsim`-style scoring is the upgrade path when users complain that `cstmrs` finds
  nothing. `ponytail:` on the scorer.
- **Index rebuilt per query.** With 10k tables that is a few milliseconds; a cache needs invalidation on
  every `conn.refresh`, which is a bug factory for a feature that does not need it yet.
- **Objects, not cells.** Searching inside row data is `grid.find` (`cmd/grid.rs:818`) and stays there; a
  global content search would need every table scanned and is not what either competitor does.
