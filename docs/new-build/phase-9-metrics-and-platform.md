# Phase 9 — Metrics board and platform

**Arc C · closes `report.designer`, `report.biDashboard`, `report.export`, `monitor.charts`, `priv.roles`, `priv.audit`, `tools.dataProfiling`, `job.agent`, `job.profiles`, `platform.i18n`, `platform.windows`, `platform.linux`, `platform.control`, `platform.cli` · depends on phase 8**

## The problem, measured

The last five percent of the two competitor feature lists, and the two places where Nook is thinner than its
own README claims:

| Fact | Evidence |
|---|---|
| Charts are live views over a query, never saved | `report.chart`, `report.aggregate`, `report.pivot` take SQL each time; no persisted report object |
| No dashboard that shows several at once | `report.biDashboard` catalogued as not implemented |
| `monitor.charts` — "the dashboard data feeds the report charts" | `docs/parity-checklist.md`; nothing renders a live series |
| No report export | `report.export` catalogued; the workaround is exporting the underlying result |
| Grants only, no role membership | `priv.users`, `priv.grants`, `priv.grant`, `priv.revoke`; `priv.roles` is a gap |
| No privilege audit | `priv.audit` catalogued |
| Jobs run only on this machine | `job.agent` catalogued; `nook-cli job serve` exists (`main.rs:1351`) but has no remote counterpart |
| No job export/import | `job.profiles` catalogued; the store is a JSON file nobody can move |
| English only | `platform.i18n` catalogued; every string is a literal in `ui/*.rs` |
| No Windows or Linux build | `platform.windows`, `platform.linux` catalogued; `README.md` says "macOS is fully supported" and the codebase has no `cfg(target_os)` at all |

## Scope

| Command | What it does | Params |
|---|---|---|
| `report.save` / `report.list` / `report.delete` | **new** — a report is `{name, sql, chart, label, options}` persisted in the store | `{"name":"revenue","sql":"…","chart":"line"}` |
| `report.designer` | **closes the Navicat id** — the save/edit surface over the three above | `{"name":"revenue","layout":{"w":6,"h":4}}` |
| `report.biDashboard` | **closes the Navicat id** — a board of saved reports plus a text block, laid out on a 12-column grid | `{"name":"ops","tiles":["revenue","errors"]}` |
| `report.export` | **closes the Navicat id** — a report or a whole board as HTML (self-contained) or CSV | `{"name":"ops","format":"html","path":"~/ops.html"}` |
| `monitor.charts` | **closes the Navicat id** — a saved report over monitor data, refreshed on a cadence | `{"report":"connections","every_s":30}` |
| `metrics.input` | **new** — a dashboard field whose value binds a parameter in the board's queries (TablePlus's "refresh on event") | `{"name":"since","value":"2026-10-01"}` |
| `priv.roles` | **closes the Navicat id** — role membership: who is in which role, grant/revoke role | `{"role":"readers","member":"analyst","action":"grant"}` |
| `priv.audit` | **closes the Navicat id** — effective privileges per object per principal, with where each came from | `{"schema":"public"}` |
| `tools.dataProfiling` | **closes the Navicat id** — per column: null share, distinct count, min/max, the five commonest values, and a shape guess (email, uuid, date, phone). One aggregate query per table, capped by `max_rows`. | `{"table":"users","columns":["email","status"]}` |
| `job.export` / `job.import` | **closes `job.profiles`** — a portable JSON bundle of jobs (no secrets) | `{"path":"~/jobs.json","ids":["nightly"]}` |
| `job.agent` | **closes the Navicat id** — push a job bundle to a remote `nook-cli job serve` over SSH, and read its log back | `{"host":"build@db1","ids":["nightly"]}` |
| `platform.i18n` | **closes the Navicat id** — a string table and a language switch; English and Indonesian shipped | `{"locale":"id"}` |
| `platform.control` | **closes the Navicat id** — the JSON control channel already exists (`control.rs::METHODS`, 16 methods, token-authenticated); the catalog still says not implemented because nothing declares it. This is `app.platform` reporting the channel's live state (port, key file path, methods) so the claim is checkable, plus a `ui.shot`-driven self-test. | `{}` |
| `platform.cli` | **closes the Navicat id** — same shape: `nook-cli` exists and is tested; `app.platform` reports the binary's version, the command count and whether it is on `PATH`. | `{}` |

## Design

### Reports are rows, and a dashboard is a list of rows

`report.*` today returns JSON built on the fly. Make a report a stored object:

```sql
CREATE TABLE IF NOT EXISTS report (
  name TEXT PRIMARY KEY, sql TEXT NOT NULL, chart TEXT NOT NULL,
  label TEXT NOT NULL DEFAULT '', options TEXT NOT NULL DEFAULT '{}', updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS dashboard (
  name TEXT PRIMARY KEY, tiles TEXT NOT NULL, updated_at INTEGER NOT NULL
);
```

`tiles` is a JSON array of `{"report":"revenue","w":6,"h":4,"x":0,"y":0}`. The layout is stored, not
computed, because a dashboard whose tiles rearrange themselves on reload is worse than one with no layout.

Rendering: the existing `report::chart_from_result` already turns a `ResultSet` into a `Chart` for seven kinds
(`report.rs:102`). A dashboard is `for tile in tiles { run its sql; chart_from_result }` — no new chart code,
which is the entire reason this phase is small.

### The metrics board is the same thing with a clock and an input

TablePlus's metrics board has three tile types: chart, table, input field; refresh after N seconds or on an
event; input fields drive query parameters. That maps exactly onto:

- chart tile → a saved report with `chart` set;
- table tile → a saved report with `chart: "table"` (already a `ChartKind`);
- input tile → `metrics.input`, whose value is injected into every tile's SQL as a named parameter (phase 6's
  parameter machinery, reused rather than re-invented);
- refresh → `monitor.charts`'s `every_s`, evaluated by the existing scheduler tick, not a new timer thread.

`monitor.charts` is therefore "a dashboard whose tiles query monitor data", not a separate feature. One
implementation, two entry points.

### Privilege audit is one query per engine, and the "where it came from" column matters

`priv.audit` returns, per object: `{object, principal, privilege, via}` where `via` is `direct`, `role:<name>`,
`public`, or `owner`. Postgres: `information_schema.role_table_grants` plus `pg_class.relowner`, and
`pg_auth_members` for role paths (recursive CTE, depth-capped at 10). MySQL: `information_schema.TABLE_PRIVILEGES`,
`SCHEMA_PRIVILEGES`, `USER_PRIVILEGES`, plus `mysql.role_edges` on 8.0+. SQLite: `Error::Unsupported` with a
sentence — a file has no principals, and an empty table would be a lie.

`priv.roles` is the write side: `GRANT <role> TO <user>` / `REVOKE`, with the identifiers validated by the same
allow-list `priv.grant` already uses (the conventions file is explicit: identifiers that cannot be parameterised
must be validated).

### Job bundles are portable and contain no secrets

`job.export` writes `{"version":1,"jobs":[…]}` with the `Job` structs minus anything credential-shaped — a job's
webhook URL stays (it is the point) but any embedded password does not, and the file says so in a `"secrets":
"omitted"` key. `job.import` validates the chain references and refuses a cycle before writing.

`job.agent` pushes that bundle with `scp` and then runs `ssh <host> nook-cli job import --path -` — reusing the
same shell-out discipline as the SSH tunnel (`engine/tunnel.rs` shells out to `ssh` so agent keys and
`~/.ssh/config` work). Reading the remote log back is `ssh <host> nook-cli job logs <id>`. Refuse when the
remote binary is missing rather than half-deploying.

### Localisation: a table, a macro, and one extra locale

Every string in the UI is a literal today. Retro-fitting a real i18n crate means touching 15,000 lines of UI and
is not what this plan is for. Instead:

- `crates/ui/src/strings.rs` — a `&'static [(&str, &str)]` table per locale, and `t(key) -> &'static str` that
  falls back to the key (which *is* the English text, so a missing entry renders the English sentence, never a
  key name).
- Convert the strings a user actually reads in the first pass: menus, dialogs, the status bar, empty states,
  error copy that the UI renders. Not the code comments, not the log lines.
- `platform.i18n` sets `settings.locale` and the UI reads it. Ship `en` and `id`.

Honest ceiling: this is not full localisation. It is the chrome in two languages, with a mechanism that makes
the rest mechanical.

### Windows and Linux: a CI matrix, and the one place the code assumes macOS

`grep -rn "cfg(target_os" crates apps` returns nothing, which is either very good or very suspicious. The
macOS-specific surfaces are the keychain (`profiles.rs:182` shells out to `security`) and `--shot`'s renderer.
So:

- `profiles.rs` gains a `keychain` module with three implementations behind one trait: `security` on macOS,
  `secret-tool` on Linux, `cmdkey`/DPAPI on Windows. Missing tool → the existing honest message
  (`profiles.rs:203` already says "this system has no OS keychain").
- `install.sh` gains a `target/release/nook.exe` branch, or a PowerShell twin; `cargo build` already works.
- `.github/workflows/ci.yml` with a `matrix: [macos-latest, ubuntu-latest, windows-latest]` running
  `cargo xtask ci` minus the parity-freshness check on non-macOS (regenerate on one platform, compare on all).
- The `platform.windows` / `platform.linux` catalog entries are marked implemented **only after the matrix has
  run green once**, with the run linked. A claim of support without a green run is exactly what the conventions
  file forbids.

## Files

| File | Work |
|---|---|
| `crates/engine/src/store.rs` | `report`, `dashboard`, `alert_rule` tables; job bundle read/write |
| `crates/engine/src/cmd/tools.rs` (Report group) | `report.save/list/delete/designer/biDashboard/export`, `monitor.charts`, `metrics.input` |
| `crates/engine/src/cmd/privs.rs` | `priv.roles`, `priv.audit` |
| `crates/drivers/src/{pg,mysql}.rs` | audit and role queries |
| `crates/engine/src/cmd/ops.rs` | `job.export`, `job.import`, `job.agent` |
| `crates/ui/src/views.rs` | dashboard grid, tile chrome, input tiles |
| `crates/ui/src/strings.rs` (new) | the string table and `t()` |
| `crates/ui/src/*.rs` | first-pass string extraction |
| `crates/engine/src/profiles.rs` | the keychain trait and three backends |
| `.github/workflows/ci.yml` | the matrix |
| `README.md`, `ROADMAP.md` | the support claim, updated only after a green run |

## Steps

1. Report rows + `report.save/list/delete`; `report.designer` over the existing chart code.
2. Dashboard: tiles, stored layout, the board view.
3. `monitor.charts` + `metrics.input` over phase 6's parameters; the scheduler tick drives refresh.
4. `report.export` (self-contained HTML: inline CSS, inline SVG charts — no external assets, because the file
   gets mailed to someone).
5. `priv.roles`, then `priv.audit` per engine, with the `via` column.
6. `job.export` / `job.import`, then `job.agent` over SSH.
7. i18n: the table, `t()` in the chrome, `id` translations.
8. Keychain trait + Linux/Windows backends; the CI matrix; run it; then update the claims.
9. Regenerate parity — this phase should take coverage past 85%.

## Tests

- Report rows: save, list, delete; a dashboard referencing a deleted report renders a tile that says the report
  is gone instead of failing the board.
- Dashboard: a stored layout is returned in the same order and geometry after a reload.
- `metrics.input`: a value flows into a tile's SQL as a bound parameter (assert the prepared statement, not a
  substituted literal).
- `report.export`: the HTML is one file with no external `src`/`href`; it contains the chart's SVG.
- `priv.audit`: a grant made directly and one inherited through a role report different `via` values; SQLite is
  `Unsupported`.
- `priv.roles`: grant then revoke round-trips; a role name with a quote is refused.
- Job bundle: export → import into a fresh store → the chain still validates; a cycle is refused on import.
- `job.agent`: a missing remote binary refuses without writing anything (simulated host).
- i18n: every key in `strings::en` exists in `strings::id` (a test, so the table cannot rot); an unknown key
  renders the English text.
- Sweep covers the new ids.

## Done when

A board of four tiles reloads with the same layout, exports as one HTML file that renders offline, and
`cargo xtask ci` is green on three operating systems.

## Ceilings

- **Charts are still rendered by `report.rs`**, seven kinds, no new visualisations.
- **Dashboard layout is stored, not dragged** — tiles get `x/y/w/h` from the editor, and a drag interaction is a
  follow-up once the data model is proven.
- **i18n covers the chrome, not every string.** Errors raised by the engine stay English; translating them means
  an error-code layer, which is a different project.
- **`job.agent` needs SSH and a `nook-cli` on the far side.** No daemon, no discovery, no auth of its own.
- **The support claims follow the CI, never the other way round.**
