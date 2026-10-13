# Nook

**Database administration for SQLite, PostgreSQL and MySQL — native, fast, and drivable by agents.**

A Navicat-style workbench in one Rust codebase: a desktop app (`nook`), a CLI (`nook-cli`), and an MCP
server for agents. Connections, browsing, data grid, SQL editor, design tools, transfer, sync, backup,
scheduler, monitor, privileges, reports, ER diagrams, and test-data generation — every action is a
registered command shared by all three surfaces.

## Features

| Area | What you get |
|---|---|
| **Connections** | SQLite files, PostgreSQL, MySQL/MariaDB; SSH tunnel through the system `ssh`; SSL modes (`disable`/`prefer`/`require`/`verify-ca`/`verify-full`); read-only profiles; passwords in the macOS login keychain; connection groups; saved profiles in JSON |
| **Browse** | Catalog read in one pass: schemas, tables, views, columns, types, defaults, comments, indexes, foreign keys, row estimates and sizes |
| **Data grid** | Paging, sort by any column, `WHERE` filter, inline cell editing with type coercion, insert/duplicate/delete/`NULL`, a pending-changes list showing the exact statements, one-transaction apply with rollback on first failure |
| **Query editor** | Multi-statement scripts, run statement at cursor, formatter, completion from the live catalog, `EXPLAIN`/`EXPLAIN ANALYZE`, history, snippets, transactions |
| **Design** | Create, copy, rename, drop tables; add/drop/rename/retype columns; nullability and defaults; indexes; foreign keys; views; object DDL; object search |
| **Transfer** | Copy tables between any two connections, across engines (SQLite ↔ PostgreSQL ↔ MySQL), with page-batched transactions and a per-table report |
| **Synchronize** | Row-level data sync by primary key (dry run first), structure comparison that emits DDL, structure sync with destructive steps gated |
| **Backup** | Portable `.sql` scripts for SQLite, `pg_dump`/`mysqldump` for servers, restore in either direction, backup list |
| **Automation** | Jobs with a schedule (`03:00`, `every 15m`, manual), one command each, in-app scheduler tick, `nook-cli job serve` for cron, run log with durations |
| **Monitor** | Process list with kill, server status, server variables with filter, dashboard of the biggest tables |
| **Privileges** | Users and roles, grants, create/drop users, set passwords, grant/revoke |
| **Reports** | Aggregates (`count`, `sum`, `avg`, `min`, `max`, `count distinct`), charts (column, bar, line, area, pie, scatter, table), pivot tables |
| **Modeler** | ER diagram from the live catalog with pan/zoom, SVG export |
| **Tools** | Deterministic test-data generation, CSV/JSON import, `.sql` replay, export to CSV/JSON/SQL/Markdown, visual query builder |
| **Agents** | MCP server over stdio (every command is a tool) and a JSON control channel for the running app |

## Prerequisites

- Rust 1.80 or newer (`rustup`)
- macOS is fully supported; the GUI uses `egui`/`eframe`
- `ssh` for tunneled connections; `pg_dump`/`mysqldump` only for those backup types

## Install

```sh
git clone https://github.com/kasfulk/nook.git && cd nook
./install.sh
```

`install.sh` builds the two release binaries and copies them into `~/.local/bin`; set
`PREFIX=/usr/local ./install.sh` to install elsewhere. It warns if the target directory is not on
`PATH`.

To build without installing instead:

```sh
cargo build --release -p nook -p nook-cli   # binaries land in target/release/
```

## Run

**Desktop app**

```sh
nook                                       # start with your saved connections
nook --demo                                # sample SQLite database in the temp directory
nook --conn postgres://localhost/shop
nook --sql "select * from users"
nook --control 7981                        # JSON control channel on port 7981
nook --shot /tmp/nook.png                  # render one frame to PNG and exit
```

From a checkout, prefix any of these with `cargo run -p nook --`, e.g. `cargo run -p nook -- --demo`.

**CLI** — the same surface from a shell:

```sh
nook-cli tables postgres://localhost/shop
nook-cli sql postgres://localhost/shop "select status, count(*) from orders group by status"
nook-cli dump postgres://localhost/shop --out backup.sql
nook-cli transfer --from sqlite:./shop.db --to postgres://localhost/stage --tables '*'
nook-cli datagen sqlite:./shop.db customers --rows 500 --seed 7
```

A connection is a saved profile name or a URI: `sqlite:./app.db`,
`postgres://user:pass@127.0.0.1:5432/shop`, `mysql://root@127.0.0.1/shop`.
Run `nook-cli help` (or `cargo run -p nook-cli -- help`) for the full command list.

**For agents**

```sh
nook-cli mcp    # MCP server on stdio; every command id is a tool
```

The control channel speaks one JSON object per line; the key is written to
`<config dir>/control-key.7981` and must be presented on every request:

```json
{"id":1,"key":"…","method":"engine.run","params":{"command":"sql.run","params":{"conn":"local","sql":"select 1"}}}
```

See [`docs/control-protocol.md`](docs/control-protocol.md).

## Development

```sh
cargo test          # whole test suite
cargo xtask ci      # fmt, clippy (warnings denied), tests, layering check, parity freshness
cargo xtask parity  # regenerate docs/parity-checklist.md
```

`crates/engine/src/catalog.rs` lists the features a Navicat-class tool is expected to have, each with
the command id that implements it. [`docs/parity-checklist.md`](docs/parity-checklist.md) is the
generated coverage report, and a test keeps it above a floor that only rises. `cargo xtask parity`
(or `nook-cli parity` from an install) prints the same numbers, area by area, with the gaps named.

## Architecture

| Layer | Crate | Job |
|---|---|---|
| L0 | `nook-core` | errors, cell type, connection profiles, identifiers and literals per dialect |
| L1 | `nook-model` | catalog, result sets, grid edit plans, structure diffs, DDL, ER layout |
| L2 | `nook-drivers` | `Driver` trait; SQLite, PostgreSQL, MySQL; transfer, sync, dump, backup |
| L3 | `nook-engine` | sessions, command registry, feature catalog, parity, jobs, data generation, reports |
| L4 | `nook-mcp` | MCP server over stdio |
| L5 | `nook-ui` | egui front end, dialogs, control channel, `--shot` |
| apps | `nook`, `nook-cli` | desktop app, command line |

Every user-visible behaviour is a `CommandSpec` in `crates/engine/src/cmd/*.rs`; the ribbon, menus,
command palette, CLI, control channel, and MCP tools all resolve the same ids. `cargo xtask layers`
enforces the layer ordering.

See [`ROADMAP.md`](ROADMAP.md) for status and [`docs/gaps.md`](docs/gaps.md) for the ranked work list.

## License

MIT OR Apache-2.0. Nook is an independent, clean-room project: no Navicat code, resources, icons, or
wording were copied. Navicat is a trademark of PremiumSoft Cybertech Ltd; PostgreSQL, MySQL, and SQLite
belong to their respective owners. Architecture conventions follow the
[WordCraft](https://github.com/storytold/wordcraft) project as a model.
