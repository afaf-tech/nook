# Phase 1 — State that survives a restart

**Arc A · closes `sql.snippets` · depends on nothing**

> Correction to the obvious reading: `sql.snippets` is a **catalog id with no registered command**. The
> commands that exist are `snippet.save` / `snippet.list` / `snippet.delete` (`cmd/app.rs:38–41`), which is why
> `docs/parity-checklist.md` still lists "Code snippets (`sql.snippets`) — built-in set plus your own" as not
> implemented. This phase registers `sql.snippets` as the catalog-facing id *and* makes the three snippet
> commands persist, which is what actually earns the tick.

## The problem, measured

Three things a user creates are held only in memory, so quitting loses them:

| Thing | Where it lives now | Evidence |
|---|---|---|
| Snippets / saved queries | `Session.snippets`, seeded from `default_snippets()` | `crates/engine/src/lib.rs:638`, `:903`; `snippet.save` pushes to the vec (`cmd/app.rs:347`) and never writes |
| Query history | `Session.history`, capped by `keep_history` | `lib.rs:870` `remember()`, no store call anywhere |
| Open tabs | `Session::new` always starts `vec![Tab::new(0, TabKind::Welcome, "Welcome")]` | `lib.rs:633` |

TablePlus and Beekeeper both keep all three. It is the single most-noticed difference between a tool
you live in and a tool you re-open. Nothing else in this plan matters if a restart wipes the session.

## Scope

| Command | Change | Params |
|---|---|---|
| `sql.snippets` | **new** — the catalog-facing id: list the snippets (with `tags` and `sql`), and accept `save`/`delete` actions so one id covers the area | `{"action":"list"}` |
| `snippet.list` / `snippet.save` / `snippet.delete` | **persist** through `store.rs`; the in-memory vec becomes a cache of the row set. They stay registered — the CLI and MCP reference them | unchanged |
| `sql.history` / `sql.clearHistory` | **persist**; `remember()` writes, `sql.history` reads back with the same shape | unchanged |
| `state.persist` | **new** — snapshot the open tabs (kind, title, connection id, editor text, table ref) into the store | `{}` |
| `state.restore` | **new** — rebuild tabs from the last snapshot; called by `Session::new`, refuses unknown connection ids by name | `{}` |
| `conn.pin` | **new** — pin/unpin a connection, persisted, painted first in the tree | `{"id":"local","pinned":true}` |
| `conn.reorder` | **new** — set the tree order explicitly (the data behind drag-and-drop later) | `{"order":["local","stage"]}` |
| `conn.setColor` | **new** — paint `ConnectionConfig.color` in the tree; the field exists and nothing reads it (`docs/architecture.md`, "A profile's colour is carried and never painted") | `{"id":"local","color":"#7aa2f7"}` |

## Design

**One store, one writer.** `crates/engine/src/store.rs` owns `nook.db` today with a single `ai_config`
table. Add tables there, not a fourth JSON file:

```sql
CREATE TABLE IF NOT EXISTS snippet (
  name TEXT PRIMARY KEY, sql TEXT NOT NULL, tags TEXT NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS history (
  id INTEGER PRIMARY KEY AUTOINCREMENT, conn TEXT NOT NULL, sql TEXT NOT NULL,
  ok INTEGER NOT NULL, elapsed_ms REAL NOT NULL, rows INTEGER NOT NULL, at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS ui_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS pinned (conn_id TEXT PRIMARY KEY, pin_order INTEGER NOT NULL);
```

`history` gets `DELETE FROM history WHERE id NOT IN (SELECT id FROM history ORDER BY id DESC LIMIT ?)`
after each insert, so `keep_history` keeps meaning what it means today without a growing file.

`store.rs`'s header comment currently says "the app's own SQLite database: where the AI configuration
lives" and carries a `ponytail:` note about having no version column. This phase is exactly the moment
that note predicted, so: add `PRAGMA user_version`, a `migrate()` that runs the statements above under
one transaction, and rewrite the header. Keep the `0600` treatment (`restrict_permissions`).

**Tabs snapshot shape.** One row in `ui_state` under key `tabs`, value is JSON:

```json
{"tabs":[{"kind":"Query","title":"Query","conn":"local","sql":"select 1","table":null}],"active":0}
```

Deliberately *not* stored: pending grid edits, result rows, scroll position. A restored tab is a place
to work, not a replay of a run. `SqlState.ran_script` is empty on restore, which is already the state
the register handles ("nothing has run").

**Restore rules.** A tab whose `conn` id is no longer in `profiles.json` restores with `conn: None` and
a log line naming the missing id — never a silent drop, never a failed start. Restoring more than 20
tabs keeps the first 20 and logs the rest, because a snapshot is a convenience and a 400-tab file is a
bug in the caller.

**Secrets.** Nothing in this phase touches the keychain. The snapshot holds SQL text, which may contain
a literal password — the store is already `0600` and the architecture note says so; say it again in the
new table's doc comment so nobody "improves" the file mode later.

## Files

| File | Work |
|---|---|
| `crates/engine/src/store.rs` | new tables, `migrate()`, `load_snippets`, `save_snippet`, `delete_snippet`, `load_history`, `push_history`, `clear_history`, `load_ui_state`, `save_ui_state`, `load_pins`, `save_pins` |
| `crates/engine/src/lib.rs` | `Session::new` loads from the store; `remember()` writes; `Snippet` gets `updated_at`; snapshot/restore of tabs |
| `crates/engine/src/cmd/app.rs` | `snippet.*` and `sql.history` go through the store; new `state.persist`, `state.restore` |
| `crates/engine/src/cmd/conn.rs` | `conn.pin`, `conn.reorder`, `conn.setColor` |
| `crates/ui/src/tree.rs` | pins first, explicit order, the colour dot (`tree.rs:405` already branches on `cfg.group`) |
| `crates/ui/src/app.rs` | call `state.persist` on a coalesced timer (2 s idle) and on close |
| `docs/architecture.md` | the store's new job, and the snapshot's ceiling |

## Steps

1. `store.rs`: migration + snippet round-trip. Test first: save, reopen the file, read back.
2. `sql.history` persistence with the `keep_history` trim. Test: 10 saves with `keep_history=3` leaves 3,
   newest first.
3. `state.persist` / `state.restore` with the unknown-connection and 20-tab rules.
4. `conn.pin` / `conn.reorder` / `conn.setColor`, and the tree painting them.
5. Wire the UI: persist on idle and on close; restore at start; the colour dot in the tree.
6. Re-measure: `cargo xtask parity`, regenerate the checklist, bump `floor_percent` if it moved.

## Tests

- `store.rs`: a snippet survives a process-shaped reopen; history trims; `user_version` migrates a file
  that has only `ai_config`.
- `lib.rs`: `Session::new` after `state.persist` restores kind, title, connection and editor text; a
  tab whose connection vanished restores with `conn: None` and logs the id.
- `cmd/app.rs`: `snippet.save` refuses empty SQL (already true) and now also survives a reload.
- `tests/`: an integration test under `crates/engine/tests/` using `NOOK_CONFIG_DIR` (the existing seam —
  see `tests/grid_edits.rs:22`) that writes with one `Session`, reads with a second.
- The junk-parameter sweep (`lib.rs::commands_run_with_junk_parameters_never_panic`) must still pass with
  the three new commands registered.

## Done when

`nook-cli snippet.save --name x --sql "select 1"` followed by a fresh `nook-cli snippet.list` shows it;
reopening the app restores the tabs; `cargo xtask ci` green; the store's header comment describes the
file that exists.

## Ceilings

- **No sync.** Pins, snippets and history are local to the machine. Export/import of the query library is
  phase 9. `ponytail:` on `store.rs`.
- **Snapshot on idle, not on every keystroke.** A crash loses up to 2 seconds of typing. Cheap, and the
  alternative is a write per keystroke — the reason the AI band's write-as-you-type is already recorded
  as a ceiling in `docs/architecture.md`.
- **No drag-and-drop.** `conn.reorder` takes an explicit order; the drag interaction is a UI-only follow-up
  once the order is trustworthy.
