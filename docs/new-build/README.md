# New build — the plan, per phase

What this is: the feature set of **Beekeeper Studio** and **TablePlus**, harvested and mapped onto Nook,
then cut into nine phases that each end with a working, tested slice. The inventory lives in
[`features.md`](features.md); every entry there carries a verdict — *have*, *gap*, or *reject* — with the
evidence that decided it.

The point is not to clone two apps. It is to take the things those two do better than a
Navicat-shaped tool, and to close them without breaking what Nook already is: one command registry,
five surfaces over it (ribbon, palette, CLI, control channel, MCP), and no feature that cannot be
driven by a script.

## Baseline, measured

Numbers below are measured in this checkout at `a66bcd2`, not from memory. Re-run the commands before
starting a phase; a stale baseline is a plan that lies.

| Fact | Value | Command |
|---|---|---|
| Registered commands | 155 | `grep -c 'CommandSpec::new(' crates/engine/src/cmd/*.rs` |
| Catalog entries | **207** | `cargo xtask parity` (or parse `catalog.rs`; a naive `grep -c '^        "x.y",$'` returns 152 because one-line entries like `("Data", "grid.formView", …)` do not match) |
| Navicat catalog coverage | 153 / 207 (73.9%) | `cargo xtask parity` |
| Coverage floor enforced by test | 55.0% | `crates/engine/src/parity.rs::floor_percent` |
| Tests | 450 | `grep -rn '#\[test\]' --include=*.rs crates apps xtask \| wc -l` |
| Rust, workspace | ~50,750 lines across 7 crates, 2 apps, xtask | `find crates apps xtask -name '*.rs' \| xargs wc -l \| tail -1` |

Baseline moved while this plan was being written: `origin/main` gained eight commits (trigger, routine and
sequence editors; the SQLite table rebuild; a splitter that survives `BEGIN IMMEDIATE` and `CASE` inside
trigger bodies; `schema.setPrimaryKey`). The numbers above are measured **after** rebasing onto `a66bcd2`,
and phase 7 below is written against what is actually left, not what was missing when the plan started.

> **A note on the catalog count.** `crates/engine/src/catalog.rs` mixes two layouts: most entries are one field
> per line, a few are collapsed onto a single line (a style change in `b6e1b03`). Any counting script must
> handle both, or it will report 152 entries and a coverage number nobody can reproduce. The number to trust is
> the one `cargo xtask parity` prints, which is why every phase ends by running it.

**Toolchain note.** This machine has no `cargo` and no `rustup` (`command -v cargo` fails; no
`~/.cargo`). So `cargo xtask ci`, `cargo test` and `cargo xtask parity` **cannot run here**. Every
phase's definition of done assumes a machine with Rust 1.80+; the plan does not pretend a green gate
that was never executed.

## The rules every phase inherits

These are [`docs/conventions.md`](../conventions.md) restated where they bite hardest, plus the one rule
this plan adds (7).

1. **Every user-visible behaviour is a `CommandSpec`** in `crates/engine/src/cmd/*.rs` — id, label,
   group, documented params, a test. The UI dispatches ids and never reaches into a driver.
2. **Never rename an id to move the parity number.** New ids that are not in the Navicat catalog are
   fine; they show up as "commands beyond the catalog" in the report.
3. **No new dependencies** for anything a page of stdlib can do. Streaming CSV, prepared parameters, hex
   and JSON pretty-printing are all in scope without a crate; fuzzy name matching is explicitly *not* built
   (phase 2 records it as a ceiling). `ponytail:` comment when a ceiling is deliberate.
4. **Hostile input stays hostile:** file contents, SQL text, cell values, params. Bound rows, quote
   identifiers, escape literals, clamp sizes.
5. **Destructive things ask first** and are refused without `confirm: true` in the engine, not only in
   the dialog.
6. **A phase is done when** `cargo xtask ci` is green, the parity floor has risen (or the report says
   why not), `docs/parity-checklist.md` is regenerated, and the UI half is verified by driving the app
   over the control channel and looking at the frames — not by reading the code.
7. **New rule for this plan — one writer per fact.** Persisted state (snippets, history, tabs, pins)
   goes through `crates/engine/src/store.rs` (the app's own `nook.db`), never a second file, never a
   second writer. Secrets stay in the login keychain and never enter that database.

## The phases

| Phase | Name | Closes (catalog ids) | Count | Depends on | Arc |
|---|---|---|---|---|---|
| [1](phase-1-state-and-persistence.md) | State that survives a restart | `sql.snippets` | 1 | — | A Daily driver |
| [2](phase-2-open-anything.md) | Open Anything (`Cmd+P`) | `tools.serverSearch` | 1 | 1 | A |
| [3](phase-3-grid-reading.md) | Reading data | `grid.blobEditor` | 1 | 1 | A |
| [4](phase-4-editing-and-review.md) | Editing and review | `grid.rawMode` | 1 | 3 | B Power |
| [5](phase-5-import-export-scale.md) | Import, export and sync at scale | `tools.importWizard`, `tools.importXml`, `tools.textImporter`, `transfer.columnMapping`, `transfer.speedTest`, `sync.dataDelete`, `sync.conflictRules`, `sync.bidirectional`, `backup.compression`, `backup.encryption`, `backup.local` | 11 | 4 | B |
| [6](phase-6-query-editor.md) | The query editor | `sql.parameterised` | 1 | 1 | B |
| [7](phase-7-schema-and-erd.md) | Schema depth and the diagram | `schema.tableRebuild`, `diagram.reverseEngineer`, `diagram.forwardEngineer`, `diagram.manualLayout`, `diagram.modelFile`, `tools.dataDictionary` | 6 | 3 | C Depth |
| [8](phase-8-async-engine.md) | A UI thread that never blocks | `sql.stop`, `sql.profiler`, `sql.planVisualiser`, `monitor.locks`, `monitor.replication`, `monitor.alerts` | 5 | 6 | C |
| [9](phase-9-metrics-and-platform.md) | Metrics board and platform | `report.designer`, `report.biDashboard`, `report.export`, `monitor.charts`, `priv.roles`, `priv.audit`, `tools.dataProfiling`, `job.agent`, `job.profiles`, `platform.i18n`, `platform.windows`, `platform.linux`, `platform.control`, `platform.cli` | 14 | 8 | C |

**Where the number lands:** 41 of the 54 open catalog ids close → **194 / 207 (93.7%)**. That figure is
arithmetic over the current catalog, not a measurement; it becomes a fact only when `cargo xtask parity`
prints it. The 13 that stay open are named below, on purpose.

Arc A is what makes the tool pleasant on a Tuesday. Arc B is what makes it trustworthy with real data.
Arc C is depth, and it is the only arc that may be cut short: phases 1–6 are the commitment, 7–9 are the
ceiling this plan names.

**Ordering is not negotiable in three places.** Phase 1 before 2 (quick open needs saved queries on
disk). Phase 3 before 4 (you cannot bulk-edit a row you cannot see). Phase 6 before 8 (async running
needs the parameterised and split-pane editor to be worth unblocking).

## What this plan refuses

Recorded here so nobody re-opens it as an oversight — reasons in [`features.md`](features.md#rejected):

- Cloud workspace sync, team folders, share links (`cloud.team`, `cloud.syncSettings`, `cloud.sharedQueries`,
  `conn.cloud`) — a server-side product, not a local tool. Phase 9 ships the *local* half only: pins, folders,
  and an exportable query library.
- A plugin system and plugin admin controls.
- DBngin, CockroachDB JWT, BedrockDB/ScyllaDB/Greengage drivers, an in-app updater (`tools.updater`).
- A second AI surface. `ai.generate` already covers prompt-to-SQL across seven providers; Beekeeper's
  AI shell is a chat transcript over the same wire format, and phase 9 does not add it.
- Query Magics as a product feature. The cheap half (alias-driven formatting in the results register)
  is a phase-3 stretch item; the `enums.json` indirection is not.

## What stays open after phase 9, and why

13 catalog ids survive the plan. Each is a decision, not an omission:

| Id | Why it stays open |
|---|---|
| `conn.sqlserver`, `conn.oracle`, `conn.mongodb`, `conn.redis` | Four engines. Each is a driver module behind the existing `Driver` trait, and each needs a live server to verify against — the same bar `docs/gaps.md` §9 sets. This is the next plan, not this one. |
| `conn.httpTunnel` | SSH covers the case; an HTTP tunnel is a server to deploy on the far side. |
| `conn.cloud` | Rejected with the cloud family. |
| `cloud.team`, `cloud.syncSettings`, `cloud.sharedQueries` | Rejected: needs accounts and a service. |
| `platform.web` | A WASM build of an app whose whole value is local files and sockets. |
| `sql.debugger` | A stored-procedure debugger needs a protocol-level debugger per engine (`pldebugger`, MySQL's `DBG`), a breakpoint UI, and a variable inspector. It is bigger than any single phase here. |
| `tools.importDbf` | Rejected: a format whose users have already migrated. |
| `tools.updater` | Rejected: `install.sh` and a git checkout are the channel; signed updates are a release-infrastructure project. |

## Progress log

| Date | Change |
|---|---|
| 2026-10-17 | Plan written. Inventory harvested from `beekeeperstudio.io/features` (plus the eight feature subpages) and `tableplus.com` + `docs.tableplus.com` (interface, filter, code review/safe mode, open anything, query editor, streaming, split panes, parameters, metrics board). Baseline measured at `e8c6041`. No code changed. |
| 2026-10-18 | Rebased onto `a66bcd2` after the push was rejected: `origin/main` had gained eight commits, three of which close ids this plan had listed as gaps. Re-measured (155 commands, 153/207, 450 tests) and rewrote phase 7, which had been built on the trigger/routine/sequence editors and the SQLite rebuild — both now shipped. Phases 1–6 and 8–9 were re-checked against the new HEAD and their premises still hold (`table.rs:53` is still one cell; no `hideColumns`, `quickLook`, `rowJson`, `copyTsv`, or partition support anywhere). |
