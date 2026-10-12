# DataCraft

<p align="center">
  <b>Database administration for SQLite, PostgreSQL and MySQL — native, fast, and drivable by agents.</b>
</p>

<p align="center">
  A Navicat-style workbench: connection tree, data grid with inline edits, query editor, object designer,
  ER diagrams, data transfer and synchronization, backup and restore, a scheduler, server monitor,
  privilege manager, reports and test-data generation. One Rust codebase, one command registry,
  a desktop app, a CLI and an MCP server.
</p>

<p align="center">
  <i>Built in the shape of <a href="https://github.com/storytold/wordcraft">WordCraft</a>: layered crates, every action a
  command with an id, and parity measured against the app it replaces.</i>
</p>

## Why it exists

Navicat is the tool most database work is done in, and it is closed, licensed per seat and moving to the cloud.
DataCraft takes the same workflow — connections on the left, data grid and SQL editor in the middle, wizards for
transfer, sync, backup and scheduling — and rebuilds it as a native app you can read, extend and drive from a
script or an agent.

## What works today

| Area | Highlights |
|---|---|
| **Connections** | SQLite files, PostgreSQL, MySQL and MariaDB; SSH tunnel through the system `ssh`; SSL with `disable`/`prefer`/`require`/`verify-ca`/`verify-full`; read-only profiles; passwords in the macOS login keychain; connection groups; saved profiles in JSON |
| **Browse** | Catalog read in one pass: schemas, tables, views, materialized views, columns, types, defaults, comments, indexes, foreign keys, row estimates and sizes |
| **Data grid** | Paging with a configurable size, sort by any column, `WHERE` filter, inline cell editing coerced to the column type, insert, duplicate, delete, `NULL`, a pending-changes list showing the exact statements, and one-transaction apply with rollback on the first failure |
| **Query editor** | Multi-statement scripts, run statement at cursor, formatter, completion from the live catalog, `EXPLAIN` and `EXPLAIN ANALYZE`, history, snippets, transactions and run-atomically |
| **Design** | Create, copy, rename and drop tables; add, drop, rename and retype columns; change nullability and defaults; create and drop indexes; add foreign keys; create and replace views; object DDL; empty a database; object search |
| **Transfer** | Copy tables between any two open connections, across engines (SQLite → PostgreSQL → MySQL), creating missing tables, options to drop or empty the target, page-batched transactions, a per-table report and a preview that only counts |
| **Synchronize** | Row-level data sync by primary key with inserts, updates and deletes (dry run first), structure comparison that emits the DDL, and structure synchronization that applies it with destructive steps gated |
| **Backup** | Portable `.sql` scripts for SQLite, `pg_dump` and `mysqldump` for the servers, restore in either direction, plus a backup list |
| **Automation** | Jobs with a schedule (`03:00`, `every 15m`, manual), one command each (query, backup, restore, transfer, sync, data generation), an in-app scheduler tick, `datacraft-cli job serve` for cron, and a run log with durations and failures |
| **Monitor** | Process list with kill, server status, server variables with a filter, and a dashboard of the biggest tables with a bar chart |
| **Privileges** | Users and roles, grants, create and drop users, set passwords, grant and revoke with the privilege list checked before the statement is built |
| **Reports** | Aggregate reports (`count`, `sum`, `avg`, `min`, `max`, `count distinct`), charts (column, bar, line, area, pie, scatter, table) and pivot tables |
| **Modeler** | ER diagram from the live catalog with pan and zoom, SVG export, per-table cards with what references what |
| **Tools** | Test-data generation that reads column names and types (deterministic by seed) with per-column overrides, CSV and JSON import with type inference and optional table creation, `.sql` replay, export to CSV/JSON/SQL/Markdown, and a visual query builder |
| **Agents** | MCP over stdio where every command is a tool, and a JSON control channel for the running app |

## Quick start

```sh
# Sample database and a first look (creates a SQLite file in the temp directory)
cargo run -p datacraft -- --demo

# Your own servers
cargo run -p datacraft -- --conn postgres://localhost/shop --conn mysql://root@127.0.0.1/shop

# The same surface from a shell
cargo run -p datacraft-cli -- tables postgres://localhost/shop
cargo run -p datacraft-cli -- sql postgres://localhost/shop "select status, count(*) from orders group by status"
cargo run -p datacraft-cli -- datagen sqlite:./shop.db customers --rows 500 --seed 7
cargo run -p datacraft-cli -- transfer --from sqlite:./shop.db --to postgres://localhost/stage --tables '*'

# For an agent
cargo run -p datacraft-cli -- mcp
```

Build release binaries with `cargo build --release -p datacraft -p datacraft-cli`.

## For agents: three surfaces, one registry

Every user-visible behaviour is a `CommandSpec` in `crates/engine/src/cmd/*.rs` with an id, a label,
documented parameters and a test. The ribbon, the menus, the command palette, the CLI, the control
channel and the MCP tools all resolve the same ids.

```sh
datacraft-cli commands                    # every id, with its parameters
datacraft-cli commands --group Transfer --json
```

MCP (`datacraft-cli mcp`) lists each command as a tool named by its id, and opens a saved
connection on demand when a call names one. The control channel (`datacraft --control 7981`) speaks
one JSON object per line:

```json
{"id":1,"key":"…","method":"engine.run","params":{"command":"sql.run","params":{"conn":"local","sql":"select 1"}}}
```

The key is written to `<config dir>/control-key.7981` and must be presented on every request. See
[`docs/control-protocol.md`](docs/control-protocol.md).

## Architecture

| Layer | Crate | Job |
|---|---|---|
| L0 | `datacraft-core` | errors, the one cell type, connection profiles, identifiers and literals per dialect |
| L1 | `datacraft-model` | catalog, result sets, grid edit plans, structure diffs, DDL, ER layout |
| L2 | `datacraft-drivers` | the `Driver` trait and the SQLite, PostgreSQL and MySQL implementations; transfer, sync, dump, backup |
| L3 | `datacraft-engine` | `Session` (connections, tabs, grid buffer, history), the command registry, the Navicat feature catalog, parity, jobs, data generation, reports |
| L4 | `datacraft-mcp` | MCP server over stdio |
| L5 | `datacraft-ui` | the egui front end, dialogs, the control channel, `--shot` |
| apps | `datacraft`, `datacraft-cli` | desktop, command line |

`cargo xtask layers` enforces the ordering; `cargo xtask ci` runs fmt, clippy with warnings denied,
the whole test suite, the layering check and the parity freshness check.

## Parity, measured

`crates/engine/src/catalog.rs` lists the features a Navicat-class tool is expected to have, each
with the command id that implements it. `cargo xtask parity` regenerates
[`docs/parity-checklist.md`](docs/parity-checklist.md), and a test keeps coverage above a floor that
only rises. Run `datacraft-cli parity` for the same numbers, area by area, with the gaps named.

## Roadmap and gaps

[`ROADMAP.md`](ROADMAP.md) has the stage, the headline numbers and the per-dimension assessment;
[`docs/gaps.md`](docs/gaps.md) is the ranked work list. The short version: alpha — the core
workflows are real and tested against live servers, and the missing pieces are named rather than
hidden.

## Licence and credits

MIT OR Apache-2.0. DataCraft is an independent, clean-room project: no Navicat code, resources,
icons or wording were copied, and every asset is drawn in code. Navicat is a trademark of PremiumSoft
Cybertech Ltd; PostgreSQL, MySQL and SQLite belong to their respective owners. The architecture and
the working conventions follow the [WordCraft](https://github.com/storytold/wordcraft) project
structure as a model.
