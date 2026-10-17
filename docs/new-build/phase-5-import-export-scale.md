# Phase 5 — Import and export at scale

**Arc B · closes `tools.importWizard`, `tools.importXml`, `tools.textImporter`, `transfer.columnMapping`, `transfer.speedTest`, `sync.dataDelete`, `sync.conflictRules`, `sync.bidirectional`, `backup.compression`, `backup.encryption`, `backup.local` · depends on phase 4**

## The problem, measured

Every import path reads the whole file into memory and every insert is its own statement:

| Fact | Evidence |
|---|---|
| CSV import loads the entire file as a `String` | `cmd/tools.rs:405` `std::fs::read_to_string(&path)` |
| Then it parses to `Vec<Vec<String>>` and reorders a second copy | `parse_csv`, then `reordered = data.iter().map(...)` (`tools.rs:~300`) |
| One `INSERT` per row, all inside **one** transaction | `load_rows`, `tools.rs:~370` loop |
| A big file is silently truncated, not refused | `data.iter().take(max)` with `max = min(settings.max_rows, max_rows param)` |
| Export runs the query through `run_sql`, bounded by `max_rows` | `tools.rs:547` `run_sql(session, index, &sql, None)` |
| No append / truncate / upsert choice | only `create_table` and `columnMapping` exist |
| Transfer already batches correctly, at 2,000 rows per transaction | `drivers/src/transfer.rs:12` `PAGE_ROWS` |

So the *transfer* path is the mature one, and the *import/export* path is the toy. Beekeeper's import/export
is explicitly built for "a file bigger than RAM, with upsert"; TablePlus's is a mapping wizard. Nook has
neither at scale, and a truncated import that reports success is the worst failure mode in the tool.

## Scope

| Command | What it does | Params |
|---|---|---|
| `tools.importCsv` (extend) | streaming read, chunked transactions, `mode: append\|truncate\|upsert` | `{"path":"big.csv","table":"users","mode":"upsert","key":["email"],"batch":2000}` |
| `tools.importXml` | **closes the Navicat id** — a flat XML file (one element per row, one child per column) through the same reader and mapping path; nested XML is refused with the reason | `{"path":"rows.xml","table":"users","row":"record"}` |
| `tools.textImporter` | **closes the Navicat id** — a fixed-width or delimiter-less text file with an explicit column-width spec | `{"path":"fixed.txt","table":"codes","widths":[3,12,8]}` |
| `tools.importJson` / `tools.importExcel` (extend) | same `mode`, `key`, `batch`; JSON accepts `.jsonl` | as above |
| `tools.importWizard` | **closes the Navicat id** — dry run: detected delimiter/encoding/header, inferred types per column, the mapping, and the first 20 rows as they *would* land. Writes nothing. | `{"path":"x.csv","table":"users"}` |
| `export.stream` | **new** — page a table or a query to CSV/JSONL without holding the result | `{"table":"orders","path":"orders.csv","format":"csv","page":5000}` |
| `tools.exportQuery` (extend) | `stream: true` routes through `export.stream` instead of `run_sql` | `{"sql":"…","path":"out.csv","stream":true}` |
| `transfer.columnMapping` | **closes the Navicat id** — the mapping editor exists in the UI, over the engine's `columnMapping` | `{"map":[{"source":"mail","target":"email"}]}` |
| `transfer.speedTest` | **closes the Navicat id** — one page timed against the destination, reporting rows/s and the batch size that would fit the configured timeout; `transfer.data` accepts the result as `batch` | `{"from":"sqlite:./a.db","to":"postgres://…","table":"orders"}` |
| `sync.dataDelete` | **closes the Navicat id** — the row sync already computes the diff; this applies its delete half, behind `confirm: true` and a dry run by default | `{"from":"sqlite:./a.db","to":"postgres://…","table":"orders","delete_missing":true,"confirm":true}` |
| `sync.conflictRules` | **closes the Navicat id** — per-column resolution when both sides changed: `source` (today's behaviour), `target`, `newer` (by a timestamp column), `skip` | `{"rules":[{"column":"total","on_conflict":"newer","by":"updated_at"}]}` |
| `sync.bidirectional` | **closes the Navicat id** — two passes with `conflictRules` deciding, never a silent last-writer-wins: the first pass reports conflicts, the second applies only the rows the rules resolved | `{"a":"sqlite:./a.db","b":"postgres://…","table":"orders","rules":[…]}` |
| `backup.compression` | **closes the Navicat id** — gzip a dump through the system `gzip` | `{"kind":"postgres","compress":true}` |
| `backup.encryption` | **closes the Navicat id** — age or GPG wrapper, refusing to run if the tool is absent | `{"kind":"postgres","encrypt":"age","recipient":"age1…"}` |
| `backup.local` | **closes the Navicat id** — the catalog note says "not implemented; the file is written where you say", which is a missing *second* location, not a missing dump. `keep: {"dir":"~/backups","copies":7}` copies the finished file and prunes older copies. | `{"kind":"sqlite","path":"a.sql","keep":{"dir":"~/backups","copies":7}}` |

## Design

### Streaming CSV, without a crate

`parse_csv(&text, delimiter)` already exists and returns `Vec<Vec<String>>`. Keep it — a test pins it — and
add a reader that yields rows one at a time over a `BufRead`:

```rust
fn rows<R: BufRead>(reader: R, delimiter: char) -> impl Iterator<Item = Result<Vec<String>>>
```

Quoted fields, doubled quotes and embedded newlines are the whole job; the existing parser already has the
state machine, so this is the same machine driven by a buffer instead of a `&str`. That is the one place a
"second version of the same job" is justified — and to keep the conventions honest, the *old* `parse_csv`
is reimplemented as `rows(text.as_bytes(), d).collect()`, so there is still exactly one parser.

Encoding: detect a UTF-8 BOM and strip it; refuse invalid UTF-8 with the byte offset rather than importing
mojibake. Full encoding detection (Latin-1, Shift-JIS) is the ceiling — say so and offer `encoding: "latin1"`
as an explicit opt-in that decodes with `String::from_utf8_lossy` on a per-byte map.

### Chunked transactions, and a real answer for failure

Today one transaction wraps the whole file: a 2-million-row import holds locks for its entire duration and
a failure at row 1,999,999 rolls back everything. New rule, taken from `transfer.rs`:

- `batch` rows (default 2,000) per transaction, `COMMIT` between batches.
- A failure rolls back **the current batch only** and stops, reporting `imported`, `failed_at_row`, and the
  statement that failed. Rows already committed stay committed — and the report says so in one sentence,
  because "import failed" after 800,000 committed rows must not read like nothing happened.
- `mode: truncate` runs `TRUNCATE`/`DELETE` **before the first batch**, inside its own transaction, and only
  after the file's header has been read successfully. A truncate that runs before the file is validated is
  how a bad path eats a table.
- `mode: upsert` requires `key: [...]` naming columns that are unique (checked against the catalog: PK or a
  unique index). It emits the dialect's own syntax — Postgres `ON CONFLICT (k) DO UPDATE SET …`, SQLite the
  same, MySQL `ON DUPLICATE KEY UPDATE …` — built in `nook_model::ddl` beside `create_table`, not inline in
  the command. No key, no upsert: refuse rather than silently degrade to insert.

### The wizard is a dry run, and it is the same code path

`tools.importWizard` calls the reader, the type inference and the mapping logic, and returns a report; it
does not open a connection write transaction at all. Concretely:

```json
{"delimiter":",","encoding":"utf-8","header":true,"rows_sampled":500,
 "columns":[{"source":"mail","target":"email","inferred":"text","nullable":false,"sample":["a@b.c"]}],
 "target_exists":true,"would_truncate":false,"preview":[[…20 rows…]],"notes":["column 'phone' is empty in every sampled row"]}
```

Type inference: an integer if every sampled value parses as `i64`, a float if every parses as `f64` with at
least one containing `.`/`e`, a bool if every value is in `true/false/0/1/t/f/yes/no`, a date/timestamp if
every value matches the ISO shape `YYYY-MM-DD` (`± T HH:MM:SS`), else text. Empty counts as nullable, not as
a type. The inferred type only matters when `create_table` is on; against an existing table the catalog's
types win and a mismatch is a **note in the wizard**, not a failure — the import still coerces per
`infer_value`.

### Export streams, so it is not bounded by `max_rows`

`export.stream` pages with `fetch_table(…, limit, offset, …)` — the driver seam that already exists — and
writes each page through a `BufWriter`, flushing per page. Formats: `csv` and `jsonl` (one JSON object per
line, which is the only JSON shape that streams). `json` (an array) is refused for streaming with a message
pointing at `jsonl`; producing a valid JSON array incrementally is possible but the trailing-comma dance is
not worth it when the consumer can read lines.

For a query (`tools.exportQuery --stream`), the SQL is wrapped as
`SELECT * FROM (<sql>) AS nook_stream` and paged with `LIMIT/OFFSET`. That is wrong for a query with its own
`LIMIT` or a volatile function — so the command says so in its `notes` and offers `sql: "…"` without
`stream: true` for exact semantics. Honest, not clever.

### Compression and encryption are child processes, not dependencies

`drivers/src/backup.rs` already shells out to `pg_dump`/`mysqldump` and resolves tools with `resolve_tool`
(`backup.rs:69`). Reuse that:

- `compress: true` → pipe the dump through `gzip -c` to `<file>.gz`, and `backup.restore` pipes `.gz` back
  through `gzip -d`. No crate, and the file is a normal `.gz` every other tool can read.
- `encrypt: "age" | "gpg"` → `age -r <recipient>` or `gpg --encrypt --recipient <id>`. If the binary is
  missing, **refuse with the install hint** — never write an unencrypted file under an encrypted name.
- SQLite backups are generated in-process (`dump_script`), so those get the same filter chain applied to the
  bytes.

Record the ceiling: this protects a file at rest, not a live connection; and the recipient's key is the
user's to manage.

## Files

| File | Work |
|---|---|
| `crates/engine/src/csv.rs` (new) | the streaming reader; `parse_csv` becomes a wrapper |
| `crates/engine/src/xml.rs` (new) | the flat-XML reader (no crate: a small pull parser over `&[u8]`, refusing nesting) |
| `crates/engine/src/cmd/tools.rs` | `mode`/`key`/`batch`, chunked transactions, `importWizard`, `importXml`, `textImporter`, `export.stream`, `stream` on export |
| `crates/engine/src/cmd/ops.rs` | `sync.dataDelete`, `sync.conflictRules`, `sync.bidirectional` |
| `crates/drivers/src/sync.rs` (or wherever the row sync lives) | delete half, conflict resolution, the two-pass bidirectional |
| `crates/drivers/src/transfer.rs` | `speedTest` (one timed page, no write) |
| `crates/engine/src/datagen.rs` or a new `infer.rs` | column type inference (shared with the wizard) |
| `crates/model/src/ddl.rs` | `upsert_statement`, `truncate_statement` per dialect |
| `crates/drivers/src/backup.rs` | compression, encryption and the `keep` copy/prune in the plan and in `execute` |
| `crates/ui/src/dialogs.rs` | the wizard: a dry-run report with a mapping editor and a preview table |
| `crates/ui/src/views.rs` | transfer view: the mapping editor over `transfer.columnMapping`; sync view: the conflict table |
| `crates/engine/tests/` | a large-file test with a generated 200k-row CSV |

## Steps

1. The streaming reader with the existing parser re-expressed over it; all existing CSV tests stay green.
2. Chunked transactions + the failure report; a test that a failure at batch 2 leaves batch 1 committed and
   says so.
3. `mode: truncate` (after header validation) and `mode: upsert` (with the unique-key check).
4. `export.stream` for `csv` and `jsonl`; refuse `json`.
5. `tools.importWizard` dry run; the inference table tests.
6. Wizard UI + transfer mapping UI.
7. Compression, encryption and `keep` in `backup.rs`, including the missing-binary refusal.
8. `transfer.speedTest`, then the sync trio (`dataDelete`, `conflictRules`, `bidirectional`) — each behind a dry
   run and `confirm: true`, each with a live two-database test.
9. XML and fixed-width importers through the same reader and mapping path.
10. Regenerate parity; eleven catalog ids leave the gap list.

## Tests

- Reader: quoted fields, doubled quotes, embedded newlines and delimiters, CRLF, BOM, a 1 MB file read in
  8 KB chunks yields the same rows as the whole-file parse.
- Chunking: 5,000 rows with `batch: 1000` → 5 commits; a failure at row 3,050 leaves 3,000 committed and the
  report says `failed_at_row: 3050`.
- Truncate: a bad header refuses before any `DELETE` runs (assert the table still has rows).
- Upsert: no `key` → refused; `key` not unique → refused naming the column; with a valid key, a re-import
  updates rather than duplicates (assert the count).
- Stream export: 100k rows to a file, memory-bounded (assert the writer flushed per page and the row count in
  the file equals the table's).
- `export.stream` with `format: "json"` → refused with the `jsonl` hint.
- Backup: `compress: true` produces a file `gzip -t` accepts; `encrypt: "age"` with no `age` binary → refused
  and no file written; `keep: {"copies":3}` after 5 dumps leaves 3 files, newest kept.
- `transfer.speedTest` reports a rate and writes nothing (assert the destination's row count is unchanged).
- Sync: a row present only in the source is deleted with `delete_missing` + `confirm`, and untouched without
  them; `on_conflict: "newer"` picks the row with the later `updated_at`; bidirectional reports a conflict it
  cannot resolve instead of picking a side.
- XML: a flat file imports; a nested file is refused naming the element; fixed-width splits on the given widths.
- Sweep covers the new ids.

## Done when

A 500 MB CSV imports without the process growing past a few MB, a re-import with `mode: upsert` does not
duplicate rows, and `export.stream` writes a file with more rows than `max_rows` while the UI stays alive.

## Ceilings

- **CSV only for streaming.** Excel is a zip of XML; streaming it needs a reader crate. `.xlsx` keeps its
  current whole-file path and the wizard says the row cap applies.
- **`jsonl`, not `json`, for streamed export.**
- **No parallel import.** One connection, one thread; the speed knob is `batch`.
- **Compression is gzip only** (`zstd` is the upgrade path), and encryption is whatever binary is installed.
