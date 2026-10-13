//! The HTTP side, done by shelling out to the system `curl`.
//!
//! Why a child process: the workspace has no HTTP client, and the tree already reaches for system
//! binaries whenever the work leaves pure Rust — `security` for the keychain, `ssh` for tunnels,
//! `pg_dump`/`psql` for backups — each with a `NOOK_*_BIN` override so the tests drive a fake.
//! `curl` is present on every target, speaks HTTPS and every provider's URL shape, and costs no
//! dependency: no tokio, no TLS crate, for a codebase that is synchronous end to end.
//!
//! **Where the API key goes.** Measured, not assumed: `curl` 8.8 does **not** expand `$VAR` or
//! `${VAR}` inside a `-K` config file — a header written that way is transmitted literally — so the
//! environment cannot carry the secret into the request. The key therefore goes in a temporary
//! config file created with mode 0600 and removed as soon as the child exits. That is deliberately
//! the same trade-off `profiles.rs` records for the SSH password, and strictly better than the
//! alternative: a `-H "Authorization: Bearer …"` argument is world-readable through `ps`.
//!
//! ponytail: config-file ceiling — the secret is on disk for the lifetime of one request. Upgrade
//! path is a native TLS client (rustls/ureq) that takes the header in memory; take it when the
//! workspace is willing to carry that dependency.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use nook_core::{Error, Result};

use crate::provider::Wire;
use crate::AiConfig;

/// The pinned Anthropic API version; a request without it is rejected.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Where a request for `cfg` goes. The user's base URL is treated as an API root: a missing
/// version segment is added, and a base that already ends with the full path is left alone so a
/// pasted endpoint is not doubled.
pub fn endpoint(cfg: &AiConfig) -> String {
    let provider = cfg.provider;
    let base = cfg.base();
    let base = base.trim_end_matches('/');
    let path = provider.path();
    if base.ends_with(path) {
        return base.to_string();
    }
    let mut root = base.to_string();
    if !root.ends_with(provider.version()) {
        root.push_str(provider.version());
    }
    format!("{root}{path}")
}

/// The headers for a request. An empty or absent key adds nothing: a local server that needs no
/// key must not be sent an empty `Authorization`.
pub fn headers(cfg: &AiConfig, key: Option<&str>) -> Vec<(String, String)> {
    let key = key.map(str::trim).filter(|k| !k.is_empty());
    let mut out = vec![("Content-Type".to_string(), "application/json".to_string())];
    match cfg.provider.wire() {
        Wire::OpenAi => {
            if let Some(key) = key {
                out.push(("Authorization".to_string(), format!("Bearer {key}")));
            }
        }
        Wire::Anthropic => {
            out.push((
                "anthropic-version".to_string(),
                ANTHROPIC_VERSION.to_string(),
            ));
            if let Some(key) = key {
                out.push(("x-api-key".to_string(), key.to_string()));
            }
        }
    }
    out
}

/// The request body. Anthropic takes the system prompt as a top-level field and has no `system`
/// role; every other provider takes it as the first message.
pub fn body(cfg: &AiConfig, system: &str, user: &str) -> serde_json::Value {
    match cfg.provider.wire() {
        Wire::OpenAi => serde_json::json!({
            "model": cfg.model(),
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
            "max_tokens": cfg.max_tokens,
            "temperature": cfg.temperature,
        }),
        Wire::Anthropic => serde_json::json!({
            "model": cfg.model(),
            "max_tokens": cfg.max_tokens,
            "temperature": cfg.temperature,
            "system": system,
            "messages": [{ "role": "user", "content": user }],
        }),
    }
}

/// The `curl` this build runs. `NOOK_CURL_BIN` points it at a fake, the way `NOOK_SECURITY_BIN`
/// does for the keychain.
pub fn curl_bin() -> String {
    std::env::var("NOOK_CURL_BIN").unwrap_or_else(|_| "curl".to_string())
}

/// The `-K` file: everything except the body, which arrives on stdin so the prompt never becomes an
/// argument.
fn rc_text(cfg: &AiConfig, key: Option<&str>) -> String {
    fn quoted(value: &str) -> String {
        value.replace('\\', "\\\\").replace('"', "\\\"")
    }
    let mut text = String::new();
    text.push_str(&format!("url = \"{}\"\n", quoted(&endpoint(cfg))));
    text.push_str("request = \"POST\"\n");
    text.push_str(&format!("max-time = {}\n", cfg.timeout_secs.max(1)));
    for (name, value) in headers(cfg, key) {
        text.push_str(&format!(
            "header = \"{}: {}\"\n",
            quoted(&name),
            quoted(&value)
        ));
    }
    text
}

/// Every non-zero exit and every unreadable payload is an error, never a panic.
pub struct Output {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Run `curl` with `stdin_body` on its standard input.
pub fn run_curl(bin: &str, args: &[String], stdin_body: &str) -> Result<Output> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::connect(format!("{bin}: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        // A write error here means curl died early; the wait below reports the real reason.
        let _ = stdin.write_all(stdin_body.as_bytes());
    }
    let out = child
        .wait_with_output()
        .map_err(|e| Error::connect(format!("{bin}: {e}")))?;
    Ok(Output {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    })
}

/// A config file the current user alone can read, removed by the caller.
///
/// The mode is set when the file is **created**, not after: between a create and a later `chmod` the
/// key inside is readable at whatever the umask allows, and a `chmod` that failed would return before
/// the caller's `remove_file` ever ran, leaving the key on disk. `create_new` refuses a path that
/// already exists, so a leftover from a killed run — or a symlink someone planted in a shared `/tmp`
/// — is never followed and never overwritten; a fresh name is taken instead.
fn write_rc(text: &str) -> Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    for _ in 0..8 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("nook-ai-{}-{n}.rc", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                if let Err(e) = file.write_all(text.as_bytes()) {
                    return Err(Error::io(format!("write {}: {e}", path.display())));
                }
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::io(format!("open {}: {e}", path.display()))),
        }
    }
    Err(Error::io("no free temporary file name for the curl config"))
}

/// The arguments `curl` is run with. The body arrives on stdin and the headers come from the config
/// file, so nothing here is a secret — which matters, because these are the only strings a `ps`
/// shows.
fn curl_args(rc: &Path) -> Vec<String> {
    vec![
        "-sS".to_string(),
        "-K".to_string(),
        rc.to_string_lossy().to_string(),
        "--data-binary".to_string(),
        "@-".to_string(),
    ]
}

/// Send one prompt and return the model's text.
pub fn chat_via_curl(
    cfg: &AiConfig,
    key: Option<&str>,
    system: &str,
    user: &str,
) -> Result<String> {
    let rc = write_rc(&rc_text(cfg, key))?;
    let payload = serde_json::to_string(&body(cfg, system, user))
        .map_err(|e| Error::internal(format!("encode request: {e}")))?;
    let out = run_curl(&curl_bin(), &curl_args(&rc), &payload);
    // The file holds the key: it goes whether the call worked or not.
    let _ = std::fs::remove_file(&rc);
    let out = out?;
    if !out.ok {
        let detail = out.stderr.trim();
        let detail = if detail.is_empty() {
            "no error message".to_string()
        } else {
            detail.to_string()
        };
        return Err(Error::connect(format!(
            "{}: curl failed: {detail}",
            cfg.provider.label()
        )));
    }
    parse_response(cfg, &out.stdout)
}

/// The provider's own error message, when the payload carries one.
pub fn error_message(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    for pointer in ["/error/message", "/error/code", "/message", "/detail"] {
        if let Some(text) = value.pointer(pointer).and_then(|v| v.as_str()) {
            if !text.trim().is_empty() {
                return Some(text.trim().to_string());
            }
        }
    }
    value
        .pointer("/error")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Pull the text out of a response, or report what the provider said instead.
pub fn parse_response(cfg: &AiConfig, body: &str) -> Result<String> {
    if let Some(message) = error_message(body) {
        return Err(Error::config(format!(
            "{}: {message}",
            cfg.provider.label()
        )));
    }
    let value: serde_json::Value = serde_json::from_str(body).map_err(|e| {
        Error::config(format!(
            "{}: the response is not JSON ({e}): {}",
            cfg.provider.label(),
            truncate(body, 200)
        ))
    })?;
    let text = match cfg.provider.wire() {
        Wire::OpenAi => value.pointer("/choices/0/message/content"),
        Wire::Anthropic => value.pointer("/content/0/text"),
    }
    .and_then(|v| v.as_str())
    .map(str::to_string);
    text.ok_or_else(|| {
        Error::config(format!(
            "{}: no text in the response: {}",
            cfg.provider.label(),
            truncate(body, 300)
        ))
    })
}

fn truncate(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}…")
}

/// One exchange with a provider. The engine takes this by trait so the commands can be tested
/// against a scripted answer instead of a network.
pub trait Chat {
    fn chat(&self, cfg: &AiConfig, key: Option<&str>, system: &str, user: &str) -> Result<String>;
}

/// The real client: the system `curl`.
pub struct CurlChat;

impl Chat for CurlChat {
    fn chat(&self, cfg: &AiConfig, key: Option<&str>, system: &str, user: &str) -> Result<String> {
        chat_via_curl(cfg, key, system, user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Provider;

    fn cfg(provider: Provider) -> AiConfig {
        AiConfig {
            provider,
            ..AiConfig::default()
        }
    }

    #[test]
    fn the_endpoint_is_base_plus_version_plus_path() {
        assert_eq!(
            endpoint(&cfg(Provider::OpenAi)),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            endpoint(&cfg(Provider::Anthropic)),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            endpoint(&cfg(Provider::Glm)),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
        assert_eq!(
            endpoint(&cfg(Provider::Ollama)),
            "http://localhost:11434/v1/chat/completions"
        );
    }

    #[test]
    fn a_custom_url_supplies_whatever_the_user_omitted() {
        let mut openai = cfg(Provider::OpenAi);
        openai.base_url = "https://proxy.internal".into();
        assert_eq!(
            endpoint(&openai),
            "https://proxy.internal/v1/chat/completions"
        );

        // A trailing slash, a bare host with the version, and a pasted full endpoint all land.
        let mut vllm = cfg(Provider::Vllm);
        vllm.base_url = "http://127.0.0.1:9000/v1/".into();
        assert_eq!(endpoint(&vllm), "http://127.0.0.1:9000/v1/chat/completions");

        let mut pasted = cfg(Provider::DeepSeek);
        pasted.base_url = "https://deepseek.com/v1/chat/completions".into();
        assert_eq!(
            endpoint(&pasted),
            "https://deepseek.com/v1/chat/completions"
        );

        let mut claude = cfg(Provider::Anthropic);
        claude.base_url = "https://gateway.internal".into();
        assert_eq!(endpoint(&claude), "https://gateway.internal/v1/messages");
    }

    #[test]
    fn the_key_rides_in_the_header_the_provider_expects() {
        let openai = headers(&cfg(Provider::OpenAi), Some("sk-1"));
        assert!(openai.contains(&("Authorization".into(), "Bearer sk-1".into())));

        let anthropic = headers(&cfg(Provider::Anthropic), Some("sk-2"));
        assert!(anthropic.contains(&("x-api-key".into(), "sk-2".into())));
        assert!(anthropic.contains(&("anthropic-version".into(), ANTHROPIC_VERSION.into())));
        assert!(!anthropic.iter().any(|(n, _)| n == "Authorization"));
    }

    #[test]
    fn no_key_means_no_auth_header_at_all() {
        for provider in [Provider::Ollama, Provider::Vllm] {
            let names: Vec<String> = headers(&cfg(provider), None)
                .into_iter()
                .map(|(n, _)| n)
                .collect();
            assert!(
                !names
                    .iter()
                    .any(|n| n == "Authorization" || n == "x-api-key"),
                "{provider:?} must not be sent an auth header"
            );
        }
        // An empty string is the same as no key.
        let empty: Vec<String> = headers(&cfg(Provider::OpenAi), Some("  "))
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert!(!empty.iter().any(|n| n == "Authorization"));
    }

    #[test]
    fn the_body_matches_the_wire_format() {
        let openai = body(&cfg(Provider::OpenAi), "sys", "hi");
        assert_eq!(openai["messages"][0]["role"], "system");
        assert_eq!(openai["messages"][1]["content"], "hi");
        assert!(openai.get("system").is_none());

        let anthropic = body(&cfg(Provider::Anthropic), "sys", "hi");
        assert_eq!(anthropic["system"], "sys");
        assert_eq!(anthropic["messages"][0]["role"], "user");
        assert_eq!(anthropic["messages"].as_array().map(Vec::len), Some(1));
        assert!(anthropic["max_tokens"].is_number());
    }

    #[test]
    fn a_response_is_read_the_way_the_provider_writes_it() {
        let openai = cfg(Provider::OpenAi);
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"SELECT 1;"}}]}"#;
        assert_eq!(parse_response(&openai, body).expect("sql"), "SELECT 1;");

        let anthropic = cfg(Provider::Anthropic);
        let body = r#"{"content":[{"type":"text","text":"SELECT 2;"}]}"#;
        assert_eq!(parse_response(&anthropic, body).expect("sql"), "SELECT 2;");
    }

    #[test]
    fn a_provider_error_becomes_the_message_the_user_reads() {
        let openai = cfg(Provider::OpenAi);
        let body =
            r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#;
        let error = parse_response(&openai, body).expect_err("an error payload is an error");
        assert!(error.to_string().contains("Incorrect API key"), "{error}");

        let html = parse_response(&openai, "<html>502 Bad Gateway</html>").expect_err("not json");
        assert!(html.to_string().contains("not JSON"), "{html}");

        let empty = parse_response(&openai, "{}").expect_err("no text");
        assert!(empty.to_string().contains("no text"), "{empty}");
    }

    #[test]
    fn the_config_file_holds_the_url_and_the_headers_but_never_the_prompt() {
        let text = rc_text(&cfg(Provider::OpenAi), Some("sk-secret"));
        assert!(text.contains("url = \"https://api.openai.com/v1/chat/completions\""));
        assert!(text.contains("Authorization: Bearer sk-secret"));
        assert!(text.contains("max-time = 60"));
        // The quote-escaping path: a key with a quote cannot break out of the config line.
        let text = rc_text(&cfg(Provider::OpenAi), Some("sk-\"x\""));
        assert!(text.contains("Bearer sk-\\\"x\\\""));
    }

    #[test]
    fn the_arguments_carry_no_secret_and_no_header() {
        // These are the strings `ps` shows, so no argument may carry the key — and no header may be
        // passed here at all, since the headers (one of which holds the key) come from the file.
        let args = curl_args(Path::new("/tmp/nook-ai-1-0.rc"));
        assert_eq!(args.first().map(String::as_str), Some("-sS"));
        assert!(args.iter().any(|a| a == "-K"), "{args:?}");
        assert!(args.iter().any(|a| a == "@-"), "{args:?}");
        assert!(
            !args.iter().any(|a| a == "-H" || a == "--header"),
            "{args:?}"
        );
        assert!(
            !args
                .iter()
                .any(|a| a.contains("Bearer") || a.contains("sk-") || a.contains("api-key")),
            "{args:?}"
        );
    }

    /// A fake `curl` in a scratch directory: it records its own arguments, a copy of the config file
    /// it was handed, that file's mode and path, and its standard input, then answers `reply` on
    /// stdout and exits `status`. The keychain tests use the same trick — a script where the real
    /// binary would be — because the transport is what carries the key and nothing else can watch it.
    fn fake_curl(dir: &Path, reply: &str, status: u8) -> PathBuf {
        let script = dir.join("curl");
        let text = format!(
            r#"#!/bin/sh
dir="{dir}"
rc=""
previous=""
for argument in "$@"; do
  case "$previous" in -K) rc="$argument" ;; esac
  printf '%s\n' "$argument" >> "$dir/argv.log"
  previous="$argument"
done
if [ -n "$rc" ]; then
  cp "$rc" "$dir/rc.copy"
  printf '%s' "$rc" > "$dir/rc.path"
  {{ stat -c %a "$rc" 2>/dev/null || stat -f %Lp "$rc"; }} > "$dir/rc.mode"
fi
cat > "$dir/body.copy"
if [ {status} -ne 0 ]; then
  printf '%s\n' "curl: ({status}) the fake failed" >&2
fi
printf '%s' '{reply}'
exit {status}
"#,
            dir = dir.display(),
        );
        std::fs::write(&script, text).expect("write the fake curl");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("make the fake curl executable");
        }
        script
    }

    /// Run `f` with the fake in place of `curl`, and hand it the directory to read. `NOOK_CURL_BIN` is
    /// process-wide, so the lock is held for as long as `f` runs and the tests here do not overlap.
    fn with_fake_curl(reply: &str, status: u8, f: impl FnOnce(&Path)) {
        static CURL_BIN: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let guard = CURL_BIN.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("nook-ai-transport-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let script = fake_curl(&dir, reply, status);
        let previous = std::env::var("NOOK_CURL_BIN").ok();
        std::env::set_var("NOOK_CURL_BIN", &script);
        f(&dir);
        // The scratch holds the prompt and the key in clear text, so it does not outlive the test.
        let _ = std::fs::remove_dir_all(&dir);
        match previous {
            Some(value) => std::env::set_var("NOOK_CURL_BIN", value),
            None => std::env::remove_var("NOOK_CURL_BIN"),
        }
        drop(guard);
    }

    fn read(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    #[test]
    fn the_transport_puts_the_body_on_stdin_and_the_key_in_a_file_only_its_owner_can_read() {
        let reply = r#"{"choices":[{"message":{"content":"SELECT 42;"}}]}"#;
        with_fake_curl(reply, 0, |dir| {
            let answer = chat_via_curl(
                &cfg(Provider::OpenAi),
                Some("sk-transport"),
                "SYSTEM TURN",
                "USER TURN",
            )
            .expect("the fake answered with SQL");

            // The answer came off the wire and was parsed the way the provider writes it.
            assert_eq!(answer, "SELECT 42;");

            // The prompt rides on stdin, so it is in no argument either.
            let body = read(dir, "body.copy");
            assert!(body.contains("SYSTEM TURN"), "{body}");
            assert!(body.contains("USER TURN"), "{body}");

            // The key is in the config file, and in no argument.
            let argv = read(dir, "argv.log");
            assert!(!argv.contains("sk-transport"), "{argv}");
            let rc = read(dir, "rc.copy");
            assert!(rc.contains("Bearer sk-transport"), "{rc}");

            // Created 0600, so there is no window in which the key is readable by anyone else.
            #[cfg(unix)]
            assert_eq!(read(dir, "rc.mode").trim(), "600");

            // And the file holding the key is gone once the child is done with it.
            let path = read(dir, "rc.path");
            assert!(!Path::new(path.trim()).exists(), "left behind: {path}");
        });
    }

    #[test]
    fn a_failing_curl_is_an_error_and_still_takes_the_file_with_the_key_with_it() {
        with_fake_curl("", 7, |dir| {
            let error = chat_via_curl(&cfg(Provider::OpenAi), Some("sk-transport"), "sys", "user")
                .expect_err("a non-zero exit is an error");
            assert!(error.to_string().contains("(7)"), "{error}");
            assert!(error.to_string().contains("OpenAI"), "{error}");

            let path = read(dir, "rc.path");
            assert!(
                !Path::new(path.trim()).exists(),
                "the key file outlived the failed call: {path}"
            );
        });
    }
}
