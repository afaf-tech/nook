# Attribution

DataCraft is a clean-room project. No asset, icon, image, font, sound or text was taken from Navicat,
PremiumSoft, or any other database tool.

## Assets

| Asset | Origin | Licence |
|---|---|---|
| The application icon | Drawn in code (a table glyph rendered by the UI, no image file) | MIT OR Apache-2.0 |
| Every icon in the interface | Unicode glyphs supplied by the operating system (`⟳`, `▤`, `🔑`, …) — no bundled icon font, no traced artwork | OS-provided |
| Diagram colours, themes, chart palette | Defined in `crates/ui/src/theme.rs` and `crates/ui/src/views.rs` | MIT OR Apache-2.0 |
| The sample database | Written for this project (`datacraft_ui::demo_session`), fictional names and amounts | MIT OR Apache-2.0 |

There are no binary asset files in this repository: no PNG, SVG, ICO, TTF or OTF. If one is ever
added, it must be original, public domain (CC0), or under an open licence that allows
redistribution, and it must be recorded in the table above in the same commit.

## Third-party crates

Ordinary dependencies, all under permissive licences (MIT, Apache-2.0, BSD, ISC, Zlib or MPL-2.0),
resolved by Cargo. The notable ones: `rusqlite` (SQLite bindings), `postgres`/`tokio-postgres`,
`mysql`, `serde`/`serde_json`, `egui`/`eframe`, `rfd`, `native-tls`, `png`.

No GPL, LGPL or AGPL code is linked.

## Names

Navicat is a trademark of PremiumSoft Cybertech Ltd, PostgreSQL of the PostgreSQL Global Development
Group, MySQL and Oracle of Oracle Corporation, and SQLite is in the public domain. These names appear
only to describe compatibility, comparison or connection targets. DataCraft is not affiliated with,
sponsored by, or endorsed by any of them.

The architecture and working conventions follow the
[WordCraft](https://github.com/storytold/wordcraft) project as a structural model (layered crates,
commands with ids, measured parity). No WordCraft or Microsoft code, text or asset was copied.
