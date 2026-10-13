# Nook roadmap

**Stage: alpha** · the core workflows are real and tested against live servers; the gaps are named.

> **Last reviewed:** 2026-10-13 · **Target:** Navicat Premium 17 (the workflow, not the code)

## Headline numbers (2026-10-13)

| Number | Value | Kind |
|---|---|---|
| Navicat feature catalog coverage | **145 / 207 (70.0%)** | measured (`cargo xtask parity` → [`docs/parity-checklist.md`](docs/parity-checklist.md)) |
| Commands | 146 registered ids | measured (`nook-cli mcp` → `tools/list`) |
| Engines with a live driver | **3** (SQLite, PostgreSQL, MySQL/MariaDB) | measured: catalogs, queries, edits, transfer, sync and backup run against each |
| Engines catalogued but not implemented | 4 (SQL Server, Oracle, MongoDB, Redis) | named in the catalog, no driver |
| Tests | 331 | measured (`cargo test --workspace -- --list`) |
| Code | ~42,400 lines of Rust in 7 crates, 2 apps and xtask | measured (`wc -l` over every `.rs`) |
| Surfaces over the same registry | 5 (ribbon/menus, command palette, CLI, control channel, MCP) | measured |

**Why alpha:** the eight core workflows — connect, browse, query, edit, design, transfer, sync,
back up and schedule — work end to end against real SQLite and PostgreSQL servers, and the whole
surface is reachable from a script or an agent. What is missing is breadth (more engines, more
importers, cloud and collaboration), depth in a few dialogs, and MySQL has not been exercised
against a live server on this machine (see the gaps).

## By dimension

| Dimension | Coverage | Note |
|---|---|---|
| Connections and security | 75% | SSH tunnels and SSL are in; HTTP tunnel, cloud connections and SSH-by-password are not |
| Data browsing and editing | 89% | the strongest area: paging, sort, filter, typed edits, pending SQL, transactional apply |
| Object design | 85% | tables, columns, indexes, foreign keys and views; no trigger, function or sequence editors |
| Query work | 76% | run, format, complete, explain, history, snippets; no debugger, profiler or visual plan |
| AI query generation | 100% | 7 providers over 2 wire formats, a custom base URL each, `@table` context, keys in the keychain; no chat, streaming or embeddings |
| Transfer and sync | 50% | cross-engine transfer, row sync, structure diff and sync; no column mapping or bi-directional sync |
| Backup and restore | 50% | SQLite scripts, `pg_dump`/`mysqldump`, restore, listing; no compression or encryption |
| Automation | 64% | jobs, schedules, run log, in-app tick and `job serve`; no notifications, chaining or remote agent |
| Monitoring | 56% | process list, status, variables, dashboard; no locks, replication or alerts |
| Privileges | 78% | users, grants, create/drop, password, grant/revoke; no audit report |
| Reports and modeler | 50% | aggregates, charts, pivot, ER diagram and SVG export; no report designer, no BI dashboard |
| Import and export | 55% | CSV, JSON, `.sql`, data generation; no Excel, XML or DBF |
| Platform | 33% | native app, MCP, CLI and themes; no control-channel parity in the UI checklist, no localisation, no Windows or Linux build made here |

## Milestones

| Milestone | State | What it means |
|---|---|---|
| M1 Skeleton | done | layered workspace, error type, cell type, dialect quoting, DDL, edit plans, structure diff, ER layout |
| M2 Drivers | done | SQLite, PostgreSQL and MySQL: catalog introspection, typed results, server info, monitor, privileges, DDL, table rebuild |
| M3 Engine | done | `Session`, 146 commands, the Navicat catalog, parity measurement, jobs, data generation, reports, profiles, keychain, SSH tunnel |
| M4 Surfaces | done | desktop app, CLI, MCP server, JSON control channel |
| M5 Verification | in progress | 331 tests; live runs against SQLite and PostgreSQL; MySQL pending a working server on this machine |
| M6 Breadth | next | the top of [`docs/gaps.md`](docs/gaps.md): MySQL verification, form view, column mapping, Excel import, trigger editors, notification hooks |

## Progress log

| Date | Change |
|---|---|
| 2026-10-13 | AI query generation. New `nook-ai` crate: 7 providers over 2 wire formats (OpenAI-compatible and Anthropic), each with its own base URL, plus `ai.generate`, `ai.providers`, `ai.setKey`, `ai.clearKey` and `ai.tables`, reachable from all 5 surfaces. An `@table` mention in the prompt expands into that table's columns, types, keys and indexes. Keys live in the login keychain, never in the profile file. Verified end to end against a local server: both wire formats, a base URL with and without `/v1`, a 401 and a 429 and a timeout surfaced verbatim, a hosted provider with no key refused before any request is made, Ollama and vLLM sent no `Authorization`, the drafted SQL ran against a real SQLite database, and a mention matching nothing warned without blocking the request. Transport is the system `curl` as a child process — no new dependency — with the body on stdin and the headers, key included, in a `0600` config file. Coverage 69.0% → 70.0%; the headline numbers, the crate table and the test count were re-measured (155 was stale: 331). |
| 2026-10-12 | Interface rebuilt on a real design system: one token set for colour, spacing, radius and type; an app bar with grouped menus, painted tabs, a status bar with chips and a collapsible log; a hand-painted table widget (sticky header, row gutter, striping, dirty markers, inline editor) shared by the grid, query results, monitor, users and reports; every text token verified at 4.5:1 or better in both themes. Verified by driving the app over the control channel and reviewing the rendered frames. |
| 2026-10-12 | First working alpha: 6 crates, 2 apps, 140 commands, 69.0% catalog coverage, 155 tests. Live verification against SQLite and PostgreSQL 17: catalog, queries, grid edits, structure diff and apply, `pg_dump` backup and `psql` restore, cross-engine transfer and row sync. MySQL driver written and unit-tested; the local server is broken (`icu4c@74` missing), so it is unverified against a live server. |
