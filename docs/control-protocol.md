# The control channel

The desktop app can be driven from a script, a test or an agent. It listens on `127.0.0.1` only, and
every request must present the key written when it started.

```sh
cargo run -p datacraft -- --control 7981
# datacraft: control channel on 127.0.0.1:7981 · key ~/Library/Application Support/DataCraft/control-key.7981
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
| `ui.setEditorText` | `{"sql":"select 1"}` | replaces the active tab's editor text |
| `ui.setTheme` | `{"theme":"light"}` | sets the preference through `app.setSetting`, so `dark`, `light` and `system` are accepted and anything else answers the command's own error; returns the settings list |
| `ui.messages` | `{}` | the last 50 messages, newest first, with their level |
| `ui.commands` | `{}` | the number of registered commands and every id |
| `ui.shot` | `{"path":"/tmp/shot.png"}` | queues a screenshot of the window; the file appears a frame later |
| `ui.quit` | `{}` | closes the app |

Anything else answers
`{"error":"unknown method 'x'; try engine.run, ui.state, …"}`.

## Driving it end to end

```sh
KEY=$(cat "$HOME/Library/Application Support/DataCraft/control-key.7981")
send() { printf '%s\n' "$1" | nc 127.0.0.1 7981; }

send "{\"id\":1,\"key\":\"$KEY\",\"method\":\"engine.run\",\"params\":{\"command\":\"grid.open\",\"params\":{\"conn\":\"local\",\"table\":\"public.users\"}}}"
send "{\"id\":2,\"key\":\"$KEY\",\"method\":\"engine.run\",\"params\":{\"command\":\"grid.editCell\",\"params\":{\"row\":0,\"column\":\"email\",\"value\":\"new@example.com\"}}}"
send "{\"id\":3,\"key\":\"$KEY\",\"method\":\"engine.run\",\"params\":{\"command\":\"grid.pending\",\"params\":{}}}"
send "{\"id\":4,\"key\":\"$KEY\",\"method\":\"engine.run\",\"params\":{\"command\":\"grid.apply\",\"params\":{}}}"
send "{\"id\":5,\"key\":\"$KEY\",\"method\":\"ui.shot\",\"params\":{\"path\":\"/tmp/after-edit.png\"}}"
```

Every command in the registry is reachable this way, with exactly the parameters
`datacraft-cli commands` documents — the control channel, the CLI, MCP and the ribbon are the same
surface.

## MCP instead

For an agent that speaks MCP, `datacraft-cli mcp` serves the same registry over stdio: `tools/list`
returns one tool per command with its documented parameters, and `tools/call` opens a saved
connection on demand when a call names one (`conn`, `from` or `to`).
