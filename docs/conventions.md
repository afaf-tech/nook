# Working conventions

The rules this codebase is written to, for anyone (or anything) contributing. DataCraft is a tool
people point at databases they care about, so the bar is "never lose data, never crash, never claim
what is not measured".

## Never crash, never lose data

- **No panics in non-test code:** no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`,
  `unimplemented!`. Production crate roots carry
  `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`;
  keep it on new crates.
- **Errors are `Result<T>`** through `datacraft_core::Error` and `?`. A feature that cannot work says
  so with `Error::Unsupported` and a sentence the user can act on — it never panics and never silently
  does nothing.
- **Input-derived values are hostile** (SQL text, cell values, file contents, imports, command
  parameters, MCP arguments): bound row counts, quote every identifier, escape every literal, clamp
  sizes, and validate the identifiers that cannot be parameterised (grants, `USE`, `KILL`).
- **Destructive operations ask first:** `schema.dropTable`, `schema.dropColumn`, `schema.truncateTable`,
  `schema.dropIndex`, `schema.dropView`, `schema.emptyDatabase`, `monitor.kill` and `priv.dropUser`
  require `"confirm": true` unless the `confirm_destructive` preference is off.
- **Grid edits are statements you can read before they run**: they accumulate in the pending list and
  apply inside one transaction that rolls back at the first failure.
- **Prove it:** a fix lands with a test. The sweep in
  `crates/engine/src/lib.rs::commands_run_with_junk_parameters_never_panic` runs every command with
  junk parameters and keeps the surface honest.

## Everything is a command

- User-visible behaviour is a `CommandSpec` in `crates/engine/src/cmd/*.rs`: an id, a label, a group,
  documented parameters, and a test. The ribbon menus, the palette, the keyboard shortcuts, the CLI,
  the control channel and MCP all dispatch by id.
- A command takes `&Params` (a JSON object) and returns a JSON summary. It never reads the UI and
  never opens a dialog from its body.
- Feature ids in `crates/engine/src/catalog.rs` are the parity checklist. Implementing a catalogued
  feature means adding a command with that exact id — never rename one to move the number.
- `cargo xtask parity` regenerates `docs/parity-checklist.md`; a test keeps coverage above a floor
  that only rises. Unimplemented features stay in the catalog with a note explaining them.

## Layering and shared code

- `cargo xtask layers` rejects a dependency that points upward: core → model → drivers → engine → mcp
  → ui → apps. Nothing below the UI depends on egui.
- The UI is thin: it reads `Session` state, paints it, and dispatches command ids. Colours live in
  `crates/ui/src/theme.rs`.
- One code path per job: `quote_ident`, `sql_literal`, `returns_rows`, `split_script`,
  `datacraft_model::ddl`, `plan_edits`, `diff`, `layout`, `dump_script`, `datagen`. Do not write a
  second version inside a command.

## Interface

- **Colour, spacing, radius and type come from `crates/ui/src/theme.rs`.** A screen that invents a
  colour or a pixel of padding is a bug; add a token instead.
- **Every text token clears 4.5:1** against every surface it can land on, in both themes. Check the
  arithmetic (relative luminance), not the look — a screenshot is a guess, the ratio is a fact.
  Disabled controls use `text_dim`, so "disabled" never means "invisible".
- **Rows are drawn by `table.rs`**, never by a fresh `egui::Grid`: sticky header, gutter, striping,
  dirty markers and the inline editor come from there, and every list in the app stays consistent.
- **A bar inside a panel allocates its rect first** (`theme::band_h`) and gives its contents a
  bounded child Ui. `ui.horizontal_centered` in an unbounded panel eats the panel's height.
- **The UI is verified by rendering it:** drive the running app over the control channel, capture a
  PNG per view, look at the picture, fix, re-shoot. Typing code that "should look fine" is not a
  check.

## Gates

Before every commit: `cargo xtask ci` — fmt, clippy with warnings denied, the workspace tests, the
layering check, and the parity freshness check. Commit after each arc of work that builds.

## Documentation

- British English, full sentences. Every generated or measured number must be reproducible with a
  command in this repository (`cargo xtask parity`, `datacraft-cli commands`,
  `grep -rn "#\\[test\\]" | wc -l`).
- Comments explain **why**; do not narrate the line below or restate the type.
- Deliberate simplifications are marked in the code with a `ponytail:` comment naming the ceiling and
  the upgrade path, and repeated in `docs/architecture.md` under "Known design choices".
