//! The JSON control channel: drive the running app from a script or an agent.
//!
//! A local TCP socket speaks one JSON object per line, like WordCraft's control channel:
//! `{"id":1,"key":"…","method":"engine.run","params":{"command":"sql.run","params":{…}}}`

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender};

use nook_core::{Error, Result};
use serde_json::{json, Value};

/// A request from the socket to the UI thread, and the reply channel back.
pub struct Request {
    pub method: String,
    pub params: Value,
    pub reply: Sender<Value>,
}

/// Handle held by the app: drains requests once per frame.
pub struct Control {
    pub port: u16,
    pub key: String,
    requests: Receiver<Request>,
    pub served: u64,
}

impl Control {
    /// Bind the socket and write the key file the client has to present.
    pub fn start(port: u16) -> Result<Control> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .map_err(|e| Error::io(format!("control channel on port {port}: {e}")))?;
        let port = listener
            .local_addr()
            .map_err(|e| Error::io(e.to_string()))?
            .port();
        let key = generate_key();
        let key_path = nook_engine::profiles::config_dir().join(format!("control-key.{port}"));
        nook_engine::profiles::ensure_dir()?;
        std::fs::write(&key_path, &key)
            .map_err(|e| Error::io(format!("write {}: {e}", key_path.display())))?;
        restrict(&key_path);
        let (tx, rx) = channel::<Request>();
        let listening_key = key.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let tx = tx.clone();
                        let key = listening_key.clone();
                        std::thread::spawn(move || serve_client(stream, tx, key));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Control {
            port,
            key,
            requests: rx,
            served: 0,
        })
    }

    /// Non-blocking: every request that arrived since the last frame.
    pub fn drain(&mut self) -> Vec<Request> {
        let mut out = Vec::new();
        while let Ok(request) = self.requests.try_recv() {
            self.served += 1;
            out.push(request);
        }
        out
    }

    pub fn key_path(&self) -> std::path::PathBuf {
        nook_engine::profiles::config_dir().join(format!("control-key.{}", self.port))
    }
}

fn serve_client(stream: TcpStream, tx: Sender<Request>, key: String) {
    let peer = stream.peer_addr().ok();
    let _ = peer;
    let reader_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut reader = BufReader::new(reader_stream);
    let mut writer = stream;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) => {}
            Err(_) => return,
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(text) {
            Ok(m) => m,
            Err(e) => {
                let _ = writeln!(
                    writer,
                    "{}",
                    json!({"id": Value::Null, "error": format!("parse error: {e}")})
                );
                continue;
            }
        };
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        if message.get("key").and_then(|k| k.as_str()) != Some(key.as_str()) {
            let _ = writeln!(
                writer,
                "{}",
                json!({"id": id, "error": format!("bad key; read {}", nook_engine::profiles::config_dir().join("control-key.*").display())})
            );
            continue;
        }
        let method = message
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string();
        let params = message.get("params").cloned().unwrap_or(json!({}));
        let (reply_tx, reply_rx) = channel();
        if tx
            .send(Request {
                method,
                params,
                reply: reply_tx,
            })
            .is_err()
        {
            return;
        }
        match reply_rx.recv_timeout(std::time::Duration::from_secs(120)) {
            Ok(result) => {
                if writeln!(writer, "{}", json!({"id": id, "result": result})).is_err() {
                    return;
                }
            }
            Err(_) => {
                let _ = writeln!(
                    writer,
                    "{}",
                    json!({"id": id, "error": "the app did not answer in time"})
                );
                return;
            }
        }
        let _ = writer.flush();
    }
}

fn restrict(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

/// 32 hex characters from the system clock and a counter: enough to keep the port from being open
/// to any local process that guesses nothing.
fn generate_key() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut state = nanos as u64 ^ (std::process::id() as u64) << 21;
    let mut out = String::with_capacity(32);
    for _ in 0..8 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.push_str(&format!("{:08x}", (state & 0xffff_ffff) as u32));
    }
    out.truncate(32);
    out
}

/// Methods the control channel accepts. Anything else is refused, not silently ignored.
pub const METHODS: &[&str] = &[
    "engine.run",
    "ui.state",
    "ui.selectTab",
    "ui.selectTable",
    "ui.setEditorText",
    "ui.setTheme",
    "ui.setAiBand",
    "ui.messages",
    "ui.shot",
    "ui.commands",
    "ui.selectResult",
    "ui.selectRange",
    "ui.quickLook",
    "ui.setResultView",
    "ui.openConnection",
    "ui.setConnectionTab",
    "ui.setConnectionField",
    "ui.quickOpen",
    "ui.quit",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_long_enough_and_unique() {
        let a = generate_key();
        let b = generate_key();
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
