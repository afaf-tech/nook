# The control channel

The desktop app can be driven from a script, a test or an agent. It listens on `127.0.0.1` only, and
every request must present the key written when it started.

```sh
cargo run -p nook -- --control 7981
# nook: control channel on 127.0.0.1:7981 · key ~/Library/Application Support/Nook/control-key.7981
```

The key file sits next to `profiles.json` and is mode `0600`. Send one JSON object per line; each
request expects one response line with the same `id`.

```json
{"id":1,"key":"<32 hex characters>","method":"engine.run","params":{"command":"sql.run","params":{"conn":"local","sql":"select 1"}}}
```

```json
{"id":1,"result":{"ok":true,"result":{"columns":["?column?"],"rows":1,"data":[[1]]}}}
```

A bad key is refused with `{"id":…,"error":"bad key; read …"}`.

## Methods

| Method | Parameters | Result |
|---|---|---|
| `engine.run` | `{"command":"<id>","params":{…}}` | `{"ok":true,"result":{…}}` or `{"ok":false,"error":"…"}` |
| `ui.state` | `{}` | active tab and connection, the connection list with open state and table counts, the tab list with row and pending counts, the control key path |
| `ui.selectTab` | `{"index":0}` | `{"ok":true,"active_tab":0}` |
| `ui.selectTable` | `{"conn":0,"schema":"main","table":"customers","tab":"columns"}` | points the object inspector at a table; `conn` defaults to the active connection and `tab` (`columns`, `indexes`, `foreign keys`, `preview`) to the one already showing |
| `ui.setEditorText` | `{"sql":"select 1"}` | replaces the active tab's editor text |
| `ui.selectResult` | `{"index":1}` | puts the results register on that statement of the last run; an index the register does not hold answers `{"error":"no result 7: the run holds 2 (0..1)"}` |
| `ui.setResultView` | `{"view":"json"}` | paints the active result as `grid` or `json`; anything else answers `{"error":"unknown view \"csv\": valid views are grid, json"}` |
| `ui.openConnection` | `{"conn":"local"}` | opens the connection editor on that profile, naming it by id as `conn.*` does; `conn` omitted or `null` opens a new one, and an id no profile holds answers `{"ok":false,"error":"no such connection 'shop'; try local, shop-staging"}` |
| `ui.setConnectionTab` | `{"tab":"TLS"}` | `{"ok":true,"tab":"TLS"}`; the tabs are `General`, `TLS`, `Tunnel` and `Advanced`, matched without case, and anything else answers `unknown tab 'Security'; try General, TLS, Tunnel, Advanced` |
| `ui.setConnectionField` | `{"field":"host","value":"db.example.com"}` | sets one field of the editor's form, by the name the editor paints it under — the 22 are `name`, `kind`, `host`, `port`, `user`, `password`, `database`, `file`, `group`, `ssl`, `read_only`, `keychain`, `timeout`, `step`, `ssh_host`, `ssh_port`, `ssh_user`, `ssh_key`, `ssh_auth`, `ssh_password`, `ssh_save_password`, `ssh_local_port` — and a value the field does not take answers `{"ok":false,"error":"field 'kind' did not accept \"cassandra\"; try name, kind, …"}`, so one bad call teaches the whole surface |
| `ui.setTheme` | `{"theme":"light"}` | sets the preference through `app.setSetting`, so `dark`, `light` and `system` are accepted and anything else answers the command's own error; returns the settings list |
| `ui.setAiBand` | `{"open":true}` | opens (or closes, with `{"open":false}`) the AI band inside the active query tab and seeds its form from the session and the live catalog; returns the form's provider, base URL, key state and mention count — never the key |
| `ui.messages` | `{}` | the last 50 messages, newest first, with their level |
| `ui.commands` | `{}` | the number of registered commands and every id |
| `ui.quickOpen` | `{"mode":"objects","query":"cust"}` | opens the palette in `objects` (default) or `commands` mode with the query typed — the `>` and `#` prefixes switch the same way from the field; returns `{"ok":true,"objects":true,"query":"cust"}` |
| `ui.shot` | `{"path":"/tmp/shot.png"}` | queues a screenshot of the window; the file appears a frame later |
| `ui.quit` | `{}` | closes the app |

Anything else answers
`{"error":"unknown method 'x'; try engine.run, ui.state, …"}`.

## Preferences

`app.setSetting` takes `{"key":…,"value":…}` and answers the whole settings object. `theme`,
`page_size`, `max_rows`, `font_size`, `confirm_destructive`, `auto_commit_edits`, `keyword_case` and
`show_system_objects` are set for the run; `density` (`compact` or `comfortable`, the design's row
height switch) is written to `profiles.json` as well and is read back on the next start, so a
scripted change outlives the process. `app.settings` echoes the same object.

## Driving it end to end

```sh
KEY=$(cat "$HOME/Library/Application Support/Nook/control-key.7981")
send() { printf '%s\n' "$1" | nc 127.0.0.1 7981; }

send "{\"id\":1,\"key\":\"$KEY\",\"method\":\"engine.run\",\"params\":{\"command\":\"grid.open\",\"params\":{\"conn\":\"local\",\"table\":\"public.users\"}}}"
send "{\"id\":2,\"key\":\"$KEY\",\"method\":\"engine.run\",\"params\":{\"command\":\"grid.editCell\",\"params\":{\"row\":0,\"column\":\"email\",\"value\":\"new@example.com\"}}}"
send "{\"id\":3,\"key\":\"$KEY\",\"method\":\"engine.run\",\"params\":{\"command\":\"grid.pending\",\"params\":{}}}"
send "{\"id\":4,\"key\":\"$KEY\",\"method\":\"engine.run\",\"params\":{\"command\":\"grid.apply\",\"params\":{}}}"
send "{\"id\":5,\"key\":\"$KEY\",\"method\":\"ui.shot\",\"params\":{\"path\":\"/tmp/after-edit.png\"}}"
```

Every command in the registry is reachable this way, with exactly the parameters
`nook-cli commands` documents — the control channel, the CLI, MCP and the ribbon are the same
surface.

## MCP instead

For an agent that speaks MCP, `nook-cli mcp` serves the same registry over stdio: `tools/list`
returns one tool per command with its documented parameters, and `tools/call` opens a saved
connection on demand when a call names one (`conn`, `from` or `to`).
