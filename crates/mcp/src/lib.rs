#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! L5: MCP server. Every Nook command is a tool, so an agent can drive the same surface the
//! ribbon does — connect, browse, query, transfer, schedule.

use std::io::{BufRead, Write};

use nook_core::{Error, Result};
use nook_engine::{find, registry, Conn, Level, Session};
use serde_json::{json, Value};

pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Serve MCP on stdin/stdout until the client closes the pipe.
pub fn serve_stdio(help: String) -> Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut session = Session::new();
    let mut opened: Vec<String> = Vec::new();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| Error::io(format!("stdin: {e}")))?;
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                let error = json!({
                    "jsonrpc": "2.0", "id": Value::Null,
                    "error": {"code": -32700, "message": format!("parse error: {e}")}
                });
                writeln!(out, "{error}").map_err(|e| Error::io(e.to_string()))?;
                out.flush().map_err(|e| Error::io(e.to_string()))?;
                continue;
            }
        };
        if let Some(response) = handle(&mut session, &mut opened, &message, &help) {
            writeln!(out, "{response}").map_err(|e| Error::io(e.to_string()))?;
            out.flush().map_err(|e| Error::io(e.to_string()))?;
        }
    }
    Ok(())
}

/// One JSON-RPC message in, one (or no) response out.
pub fn handle(
    session: &mut Session,
    opened: &mut Vec<String>,
    message: &Value,
    help: &str,
) -> Option<Value> {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or(json!({}));
    let reply = |result: Value| Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    let fail = |code: i64, text: String| {
        Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": text } }))
    };
    match method {
        "initialize" => reply(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "nook", "version": env!("CARGO_PKG_VERSION") },
            "instructions": help,
        })),
        "notifications/initialized" | "notifications/cancelled" => None,
        "ping" => reply(json!({})),
        "tools/list" => reply(json!({ "tools": tool_list() })),
        "tools/call" => {
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if find(name).is_none() {
                return reply(json!({
                    "content": [{ "type": "text", "text": format!("unknown tool '{name}'; call tools/list") }],
                    "isError": true
                }));
            }
            match call_tool(session, opened, name, arguments) {
                Ok(result) => {
                    let text = serde_json::to_string_pretty(&result)
                        .unwrap_or_else(|_| result.to_string());
                    reply(json!({
                        "content": [{ "type": "text", "text": text }],
                        "structuredContent": result,
                        "isError": false
                    }))
                }
                Err(e) => reply(json!({
                    "content": [{ "type": "text", "text": e.to_string() }],
                    "isError": true
                })),
            }
        }
        other => fail(-32601, format!("method not found: {other}")),
    }
}

/// Run a tool, opening the connection it names when it is not open yet.
pub fn call_tool(
    session: &mut Session,
    opened: &mut Vec<String>,
    name: &str,
    arguments: Value,
) -> Result<Value> {
    let mut map = match arguments {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => {
            return Err(Error::config(format!(
                "arguments must be an object, got {}",
                kind_of(&other)
            )))
        }
    };
    // Point the command at a connection, opening a saved profile on demand.
    if !map.contains_key("conn") && !map.contains_key("from") {
        if let Some(default) = session.active_conn.map(|i| session.conns[i].cfg.id.clone()) {
            map.insert("conn".into(), json!(default));
        }
    }
    let mut needed: Vec<String> = Vec::new();
    for key in ["conn", "from", "to"] {
        if let Some(id) = map.get(key).and_then(|v| v.as_str()) {
            if !id.is_empty() {
                needed.push(id.to_string());
            }
        }
    }
    for id in needed {
        if opened.contains(&id) {
            continue;
        }
        let saved = session
            .profiles
            .get(&id)
            .cloned()
            .ok_or_else(|| Error::not_found(format!("no saved connection '{id}'")))?;
        if session.conn_index(&id).is_err() {
            session.conns.push(Conn::new(saved));
        }
        let index = session.conn_index(&id)?;
        nook_engine::cmd::open_now(session, index)?;
        opened.push(id);
    }
    let result = session.run(name, Value::Object(map))?;
    let errors: Vec<String> = session
        .messages
        .iter()
        .filter(|m| m.level == Level::Error)
        .map(|m| m.text.clone())
        .collect();
    if errors.is_empty() {
        Ok(result)
    } else {
        Ok(json!({ "result": result, "messages": errors }))
    }
}

/// Tool schemas: the command's own parameter documentation, plus the connection names to pick from.
pub fn tool_list() -> Vec<Value> {
    let store = nook_engine::ProfileStore::load().unwrap_or_default();
    let names: Vec<String> = store.list().iter().map(|c| c.id.clone()).collect();
    let hint = if names.is_empty() {
        "no saved connections yet; create one with conn.new first".to_string()
    } else {
        format!("saved connections: {}", names.join(", "))
    };
    registry()
        .iter()
        .map(|spec| {
            let properties = if spec.params.is_empty() {
                json!({})
            } else {
                // The documented example is a JSON object; publish its keys as the schema.
                match serde_json::from_str::<Value>(spec.params) {
                    Ok(Value::Object(map)) => {
                        let props: serde_json::Map<String, Value> = map
                            .iter()
                            .map(|(k, v)| {
                                (
                                    k.clone(),
                                    json!({
                                        "description": format!("example: {}", short(v)),
                                        "type": json_type(v),
                                    }),
                                )
                            })
                            .collect();
                        Value::Object(props)
                    }
                    _ => json!({}),
                }
            };
            json!({
                "name": spec.id,
                "description": format!("{} — {}. {}", spec.label, spec.group, if spec.help.is_empty() { hint.clone() } else { format!("{} {}", spec.help, hint) }),
                "inputSchema": {
                    "type": "object",
                    "properties": properties,
                    "additionalProperties": true,
                }
            })
        })
        .collect()
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn short(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() > 60 {
        format!("{}…", text.chars().take(59).collect::<String>())
    } else {
        text
    }
}

fn kind_of(value: &Value) -> &'static str {
    json_type(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_reports_capabilities() {
        let mut session = Session::new();
        let mut opened = Vec::new();
        let response = handle(
            &mut session,
            &mut opened,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            "help",
        )
        .expect("response");
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(response["result"]["serverInfo"]["name"], "nook");
        assert!(response["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_list_covers_the_registry() {
        let tools = tool_list();
        assert_eq!(tools.len(), registry().len());
        let run = tools
            .iter()
            .find(|t| t["name"] == "sql.run")
            .expect("sql.run tool");
        assert_eq!(run["inputSchema"]["type"], "object");
        assert!(run["inputSchema"]["properties"]["sql"].is_object());
    }

    #[test]
    fn notifications_get_no_response() {
        let mut session = Session::new();
        let mut opened = Vec::new();
        assert!(handle(
            &mut session,
            &mut opened,
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            "help"
        )
        .is_none());
    }

    #[test]
    fn unknown_method_and_tool_are_errors_not_panics() {
        let mut session = Session::new();
        let mut opened = Vec::new();
        let response = handle(
            &mut session,
            &mut opened,
            &json!({"jsonrpc": "2.0", "id": 2, "method": "nope"}),
            "help",
        )
        .expect("response");
        assert_eq!(response["error"]["code"], -32601);
        let response = handle(
            &mut session,
            &mut opened,
            &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": {"name": "not.a.command", "arguments": {}}}),
            "help",
        )
        .expect("response");
        assert_eq!(response["result"]["isError"], true);
    }

    #[test]
    fn calling_a_command_without_a_connection_reports_the_error() {
        let mut session = Session::new();
        let mut opened = Vec::new();
        let response = handle(
            &mut session,
            &mut opened,
            &json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                    "params": {"name": "sql.run", "arguments": {"sql": "select 1"}}}),
            "help",
        )
        .expect("response");
        assert_eq!(response["result"]["isError"], true);
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default();
        assert!(text.contains("connection"));
    }

    #[test]
    fn app_about_works_over_mcp() {
        let mut session = Session::new();
        let mut opened = Vec::new();
        let out = call_tool(&mut session, &mut opened, "app.about", json!({})).expect("about");
        assert_eq!(out["name"], "Nook");
    }
}
