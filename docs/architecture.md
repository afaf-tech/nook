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
  `ui.selectResult` and `ui.setResultView` (so every statement of a run and both result views can be
  shot), `ui.setTheme` (so both palettes can be shot without touching the window), `ui.shot` per view,
  `ui.quit`. The frames are then reviewed as images, and anything visual is fixed and
  re-shot. `ui.shot` writes the file and leaves the app running; only a `--shot` launch quits after
  capture.

## Known design choices and their ceilings

- **The editor pane carries no running state.** The design's `.prog` hairline, its `Executing…` pill
  and its running footer are not painted: `sql.run` is a blocking call on the UI thread, so a frame
  that says a statement is running would be a frame the app cannot draw while it runs. `Stop` exists
  and is painted disabled, with the command registry's own `sql.stop` help as its hover text, so the
  control cannot describe the engine differently from the engine. The ceiling is a driver-level cancel
  with a job id the UI can poll; until then the run is bounded by `max_rows` (see `docs/gaps.md` §7).
- **The register has two views, grid and JSON.** `docs/edge-cases.md` and the design's mockup show a
  `Text` view too; it is not built, and `ui.setResultView` lists only the two it accepts. The upgrade
  path is one more `View` arm beside the JSON one, which is the same serialisation with different
  wrapping.
- **A result tab cannot be closed.** The register's tab row is one tab per statement of the last run —
  the run's own shape — so closing one would ask what the editor should then run. Re-running a script
  reopens every tab it holds; the upgrade path is a per-tab `closed` flag the next run respects.
- **The editor's toolbar is a cut-down version of the design's.** The design's full set plus its
  connection chip does not fit the pane's 776px, measured by the test at the same sizes: `Chart`,
  `Pivot`, `Save snippet` and `Export ▾` are dropped, and the connection chip with them, because the
  status bar already names the connection. The kept set is `Run`, `Run statement`, `Stop`, `Explain`,
  `Format`, `Clear` and the caret read-out. The ceiling: an overflow menu, or icons instead of labels,
  when one of the dropped actions is asked for.
- **History's surface is gone.** The editor's `History` band (`.hist`, the run list the mockup shows
  above the register) is not painted: `SqlState.results` is the current run only, and the register's
  tab row already names each statement's outcome. The ceiling is a session-level run log, which the
  engine would have to keep.
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
- **Painted activity glyphs.** The rail's five icons are hand-drawn into a `Rect` from the design's
  own SVG paths (`ui::icons`), so there is no icon font and no asset bundler; the ceiling is that each
  new glyph is code, and a second icon family with different stroke rules would need a real icon
  source. `ponytail:` the sampler handles the three-segment cubics the design uses and nothing else.
- **Rail semantics.** The rail speaks the app's verbs, not the mockup's marketing copy: a report is a
  chart built from a query result, and there is no "new report" verb, so Reports selects an existing
  report tab and is disabled when none is open. Fake workspaces would have needed a state the engine
  does not have.
- **No loading skeleton in the grid.** The design draws a `.sk` state (8px `surface_3` bars inside the
  row rhythm, pager disabled, a ghost Cancel) for a page that is on its way. Every load in this app is
  synchronous — `grid.rs` reads a page, a count or a schema and returns with the result in the same
  frame — so no frame exists in which the grid has no rows *and* knows that more are coming. Painting a
  skeleton from a flag that is set and cleared inside one frame would be a state nothing can ever show.
  The upgrade path is real asynchrony (a job that reports its own progress); the design's state can be
  added then with the flag it needs.
- **The workbench pane is flush and each surface carries its own insets.** `app::show` gives the
  central pane a frame with no margin — the design's `.reg-grid` has none either — so a surface that
  needs air inside the pane states it itself (`views::welcome`, the query editor and the monitor
  dashboard have no such rule in the design, so they sit flush and were re-measured rather than
  padded). Inside the grid, `show` zeroes `item_spacing.y` so its bands stack hairline to hairline.
- **The grid asks the widget for its head alone on an empty page.** `TableView::height = 0` is the
  sentinel: `table::show` paints the head inside a horizontal scroll area and returns, and the caller
  owns the space under it (the `.empty` block's own `padding:28px`). The head keeps that scroll area
  because a table wider than the pane would otherwise be clipped with no bar to reach the columns past
  the edge. Its height is the head's own, so the block still lands one head below the hairline — the
  scroll area collapses to its content, and the horizontal bar is charged to the width, not the height,
  so an overflowing head does not push the block down either. The viewport's floor is `HEAD_H` rather
  than egui's `min_scrolled_size` default (64px), which is what put the block 72px down before; the
  floor is idle while egui's bars float, and holds the band if a future style stops them floating.
  `ponytail:` a zero-height body is not expressible
  — nothing asks for one — so the upgrade path is a `Body` enum (`HeadOnly | Scroll(f32)`) if a caller
  ever wants a body of no height.
- **The grid's badge and notice marks have one definition each.** The head reserves
  `theme::pk_badge_width` and paints through `theme::paint_pk_badge`, so the toolbar's `.pkbadge` and
  the head's cannot drift apart; `theme::notice_band` takes the band's leading `icons::Glyph` and its
  message `FontId`, which is how the `.errband` gets `.ei` plus a `--t-sm` mono message and the
  `.robanner` gets `.ri` plus its proportional `--t-xs` paragraph.
- **One `editable` flag decides what the grid offers.** `GridState::editable` is false when the engine
  would refuse an edit (`grid.editCell`, `grid.setNull` and the rest go through `pk_values`), and every
  surface that names such an action reads it: the toolbar's `+ Row` / `Duplicate` / `Delete`, the
  selected-cell band's `Double-click` keycap and `Set NULL`, and the widget's own inline editor and
  dirty stripe. A surface that forgot it would advertise an action the engine rejects.
- **The grid prints only the facts it holds.** `.matches` shows the count `grid.countRows` returned —
  and only until the grid starts asking for different rows: `grid.filter`, `grid.clearFilter`,
  `grid.sort` and `grid.apply` drop it, because a count read under the previous request is not this
  page's match total (an applied edit inserts, updates or deletes rows of the set just counted).
  With no count there is nothing honest to print but the page in hand (`50 rows on this page`). The
  `.robanner`'s reason comes from the binding and the schema — not from the page — in the order
  `GridState::editable` checks them, so a keyless table is never told it has no primary key because a
  page came back empty.
- **Bar bands.** `theme::bar_frame` contributes no stroke and no margin, and every panel is built with
  `show_separator_line(false)`, so a bar's height is its token plus the hairline painted inside it.
  The ceiling is that any future panel must paint its own edges to be visible.
- **Density reaches the data grid and the results, not the catalog lists.** `theme::row_h` turns the
  persisted `density` preference into 26px compact or 30px comfortable rows, and only the two call
  sites that render a real grid decide per frame (`grid.rs`, `editor.rs`); `views::list` pins
  `theme::ROW_H`. The design demonstrates comfortable rows on its grid register, so the lists keep the
  compact measure until an explore register asks otherwise — then `list` takes a row height too.
- **The explore preview is a page, not a pager.** `datacraft_engine::cmd::preview_table` runs the same
  `page_select` the grid does, without a history entry and without registering a command of its own, so
  `cargo xtask parity` stays honest. `ponytail:` there is no control-protocol surface for it and no
  second page; add a command and an offset argument when the preview needs to scroll past
  `PREVIEW_ROWS`.
- **The inspector's Foreign-keys tab states the outgoing half only.** Listing what references a table
  needs a scan no command performs, so the model paints the `Outgoing` group and, when it is empty,
  the design's sentence rather than a second group it cannot fill. `ponytail:` add a
  `schema.scanReferences` command and a second `Group` when the incoming half is worth a round trip.
- **The `.fchips` add chip has a solid border.** The design draws `border-style: dashed`; egui has one
  stroke per shape and no dash pattern, so the chip keeps the design's weight, radius and `border`
  colour with a solid hairline. `ponytail:` the ceiling is that the chip is not visually distinct from
  a real filter chip at a glance; the upgrade path is painting the dashes by hand along its top and
  bottom edges.
- **The `.stalestrip` carries the last order and filter, not the last rows.** `load_grid` clears
  `GridState::result` when the page fails, so the strip can say what the grid is still asking for but
  the design's `.stale` block — the previous page kept on screen under the error — has no rows to
  paint. `ponytail:` the upgrade path is keeping the previous `ResultSet` in `GridState` and painting
  it inside a `.stale` wrapper.
- **One changes surface, two thirds of the design's.** The design draws both the toolbar's `.pend`
  cluster and a drawer with a `.dfoot`; this app keeps the persistent `.pend` cluster (only while the
  buffer is non-empty, as the design's own `0 pending` states do) and the drawer, whose footer now
  carries `Copy to SQL`, Revert and `Apply n changes`. `Copy to SQL` writes the statements one per
  line, because each `edit.sql` already ends in `;` and a `;` separator would leave an empty statement
  between every pair. `ponytail:` the drawer body is still the
  `CollapsingHeader` of `kind · sql` lines rather than the design's grouped `.dgrp`/`.dchg` list; the
  grouped list needs per-kind headers and a per-change undo, which the engine's `Vec<PendingEdit>` can
  address when the drawer is worth the second pass.
