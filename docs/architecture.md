# DataCraft architecture

> **Last reviewed:** 2026-10-12 · **Target:** Navicat Premium 17 workflows

How DataCraft is built today. The conventions for working in the code are in
[`conventions.md`](conventions.md).

## Layers

~25,500 lines of Rust in 6 crates, 2 apps and `xtask`. `cargo xtask layers` enforces the ordering:
a crate may only depend on crates below it, and nothing below L5 depends on egui.

| Layer | Crate | Lines | Job |
|---|---|---|---|
| L0 | `datacraft-core` | 1.0k | `Error`/`Result`, `Value` (the one cell type), `ConnectionConfig` with SSH and SSL settings, dialect quoting and literals, `returns_rows` |
| L1 | `datacraft-model` | 1.8k | `Catalog`, `Schema`, `Table`, `Column`, `Index`, `ForeignKey`, `DbProcess`, `DbUser`, `ServerInfo`, `ResultSet`; grid edit plans; DDL for all three dialects; structure diff; ER layout |
| L2 | `datacraft-drivers` | 3.8k | `Driver` trait + `sqlite`, `pg`, `mysql`; statement splitter; data transfer; row-level sync; `.sql` dump; backup and restore through the vendor tools |
| L3 | `datacraft-engine` | 10.1k | `Session` (connections, tabs, grid buffer, history, messages), 140 commands in `cmd/*.rs`, the Navicat feature catalog, parity measurement, jobs and the scheduler, data generation, reports, profiles and the keychain, the SSH tunnel |
| L4 | `datacraft-mcp` | 0.3k | MCP server over stdio: every command is a tool, connections open on demand |
| L5 | `datacraft-ui` | 6.9k | egui front end: design tokens, table widget, app shell, sidebar, grid, editor, designer, diagram, charts, monitor, jobs, dialogs, control channel, `--shot` |
| apps | `datacraft`, `datacraft-cli` | 1.3k | desktop binary and the command line |

## Data flow

1. **Connect.** `conn.open` starts the SSH tunnel if the profile asks for one (`ssh -N -L`, so agent
   keys and `~/.ssh/config` work), opens the driver, reads `ServerInfo` and then the whole catalog in
   one pass. The catalog is cached on the connection for the tree, completion, the diagram and every
   DDL generator.
2. **Act.** Every user-visible behaviour is a `CommandSpec` with an id. The ribbon, the palette, the
   keyboard, the CLI, the control channel and MCP resolve the same id through `Session::run`,
   which checks availability, runs the command and logs failures. Commands never touch the UI.
3. **Read data.** The grid asks for one page (`SELECT … LIMIT n OFFSET m`, with the tab's `ORDER BY`
   and `WHERE`), and the driver bounds the result at `max_rows`.
4. **Edit data.** A cell edit is coerced to the declared column type, turned into an `UPDATE` keyed by
   the primary key (`datacraft_model::plan_edits`) and pushed onto the tab's pending list. `grid.apply`
   runs the whole list inside one transaction and rolls back at the first failure.
5. **Move data.** Transfer reads pages of 2,000 rows and writes each page inside a transaction
   (`transfer::transfer`); sync compares both sides by primary key (`transfer::compare_rows`, a pure
   function) and writes only the differences; structure sync compares two catalogs
   (`datacraft_model::diff`) and applies the DDL, with destructive steps gated behind a flag.
6. **Back up.** SQLite is dumped in process to a portable `.sql` script (`transfer::dump_script`);
   PostgreSQL and MySQL shell out to `pg_dump`/`mysqldump` with the password in the environment, never
   on the command line. Restore mirrors both paths.
7. **Automate.** A job is a command plus a schedule, stored as JSON. The desktop app ticks every 30
   seconds and `datacraft-cli job serve` does the same for cron; both call `job.runDue`, which
   dispatches each due job through the registry and records the result.

## Interface

The front end is one design system, not a set of screens that each look after themselves.

- **`theme.rs` owns every visual decision** — colour, spacing, radius, type size, plus the small
  widgets the rest of the UI composes: `band`/`band_h` (a fixed-height horizontal bar),
  `card_frame`, `chip`, `ghost_button`, `primary_button`, `segmented`, `meter`, `keycap`, `divider`.
  No screen invents a colour or a pixel of padding.
- **Text contrast is a floor, not a taste:** every text token clears 4.5:1 against every surface it
  sits on in both themes (`text_dim` 6.6–7.5:1, `text_faint` 4.8–5.8:1). Disabled controls stay
  readable: they read as "not now", never as "not drawn".
- **`table.rs` is the only table.** Sticky header with sort carets and a `PK` tag, row-number gutter,
  zebra striping, hover and selection, amber dirty markers on pending cells, per-cell hit testing,
  and an inline editor (`Enter` commits, `Esc` cancels). The data grid, query results, monitor lists,
  the users list and report tables all use it, so they behave identically.
- **`band_h` exists because of a trap:** `ui.horizontal_centered` inside an unbounded panel expands to
  the panel's whole height, which starves everything below it of a clip rect and paints nothing. Any
  bar that sits in a panel allocates its rect first, then hands a bounded child Ui to its contents.
- **The app shell** is an app bar (identity, grouped menus built from the command registry groups,
  palette field), a tab strip (painted, accent underline, middle-click to close), the view's own
  toolbar, the sidebar, and a status bar of chips (connection, tab, pending edits, messages,
  coverage, control channel) with the log as a collapsible drawer behind the messages chip.

## Driver model

One trait, `Driver`, with defaults for everything a given engine cannot do — SQLite returns
`Error::Unsupported` for processes, users and database switching, and the message says so. Three
implementations:

- **SQLite** over `rusqlite` (system libsqlite3). Introspection through `sqlite_master` and the
  `table_info`/`index_list`/`index_info`/`foreign_key_list` pragmas; table rebuilds for DDL that
  SQLite cannot do in place.
- **PostgreSQL** over the simple query protocol on purpose: values come back as text, so `numeric`,
  `int8` and `timestamptz` keep full precision without a per-type decoder, and multi-statement
  scripts behave the way the editor submits them. Column names and types come from a prepared (but
  unexecuted) statement. TLS through native-tls.
- **MySQL/MariaDB** over `mysql`. Result metadata gives names and types, `information_schema` gives the
  catalog, `SHOW CREATE TABLE` gives DDL, and the MySQL-specific boolean and date shapes are mapped in
  one function.

## Testing

- 155 `#[test]` functions across the workspace: quoting and literals, catalog types, DDL per dialect,
  structure diff, ER layout, statement splitting, edit planning, data generation determinism, report
  shaping, schedules, profiles, MCP JSON-RPC, and every command against junk parameters
  (`commands_run_with_junk_parameters_never_panic`).
- Live checks that need a server are run with the CLI rather than in the unit suite, because they
  mutate databases:
  - PostgreSQL 17: `datacraft-cli tables|sql|describe|ddl|monitor|users`, transfer SQLite → PostgreSQL,
    `sync-data` dry run and apply, `diff-structure` and `--apply`, `dump` (pg_dump) and `restore` (psql)
    into a fresh database.
  - MySQL/MariaDB: the same script, once a server is reachable.
- `cargo xtask ci` runs fmt, clippy with warnings denied, the workspace tests, the layering check and
  the parity freshness check.
- UI verification: `cargo run -p datacraft -- --shot /tmp/shot.png` renders a frame and writes a PNG.
  For a real check, start `datacraft --demo --control <port>` and drive it over the control channel:
  `ui.selectTab` (a negative index shows the start page), `engine.run`, `ui.setEditorText`,
  `ui.setTheme` (so both palettes can be shot without touching the window), `ui.shot` per view,
  `ui.quit`. The frames are then reviewed as images, and anything visual is fixed and
  re-shot. `ui.shot` writes the file and leaves the app running; only a `--shot` launch quits after
  capture.

## Known design choices and their ceilings

- **Text protocol for PostgreSQL.** `numeric` precision is exact and the code is small; the price is
  no binary results and no bind parameters. A prepared path can be added behind the same trait when a
  feature needs it.
- **Client-side literals for edits and transfers.** One function (`sql_literal`) escapes values for the
  grid, the import wizard, the transfer, the sync and the dump, which is why they share one code path.
  Parameterised execution is the upgrade path for very large or binary-heavy loads.
- **Generated DDL for PostgreSQL.** Postgres has no `SHOW CREATE TABLE`, so `schema.ddl` builds the
  statement from the catalog: indexes and foreign keys included, formatting not identical to
  `pg_dump`. MySQL and SQLite ask the server.
- **ASCII-safe SVG diagrams.** No external fonts, no scripts, escaping tested.
- **Chip weight.** The design gives chips a 600 weight; egui's bundled proportional family has one
  weight, so chips read by colour, size and a pill radius instead. Bundling a bold family is the
  upgrade path if the weight ever has to carry meaning.
