//! AI commands: draft SQL from a sentence, with `@table` mentions pulling in a table's structure.
//!
//! The provider, model and endpoint come from the AI settings (`app.setSetting ai_*`); the key comes
//! from the keychain or `NOOK_AI_KEY` — never from the profile file. Nothing here holds a key in a
//! value it returns, so an agent reading a command's output can never read a secret.

use nook_core::{Dialect, Error, Result};
use serde_json::json;

use crate::{CommandSpec, Level, Params, Session, TabKind, TableRef};

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("ai.generate", "Generate SQL", "AI", generate)
            .params(r#"{"prompt":"users who signed up this week, with their order count","tables":["users"],"open_tab":true}"#)
            .help("Draft SQL from a sentence. An @table mention in the prompt pulls that table's columns, keys and foreign keys into the request. Provider and model come from the AI settings; the key comes from the keychain. open_tab puts the result in a new query tab."),
        CommandSpec::new("ai.providers", "AI providers", "AI", providers)
            .help("Every provider Nook can talk to with its default endpoint and whether a key is set. Never echoes the key."),
        CommandSpec::new("ai.setKey", "Save an API key", "AI", set_key)
            .params(r#"{"provider":"openai","key":"sk-…"}"#)
            .help("macOS: stored in the login keychain under ai.<provider>, never in the profile file. Omit \"provider\" for the active one. Elsewhere set NOOK_AI_KEY instead."),
        CommandSpec::new("ai.clearKey", "Forget an API key", "AI", clear_key)
            .params(r#"{"provider":"openai"}"#),
        CommandSpec::new("ai.tables", "Tables for the prompt", "AI", tables)
            .params(r#"{"filter":"user"}"#)
            .help("The candidates an @ mention can resolve to, from the live catalog."),
    ]
}

fn generate(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    generate_with(session, p, &nook_ai::CurlChat)
}

/// The dialect a prompt is written for: the connection's own, or SQLite when nothing is open, so a
/// prompt can be drafted before connecting.
fn connection_dialect(session: &Session, index: usize) -> Dialect {
    session
        .conns
        .get(index)
        .and_then(|conn| conn.driver.as_ref().map(|d| d.dialect()))
        .unwrap_or(Dialect::Sqlite)
}

fn provider_from_params(session: &Session, p: &Params) -> Result<nook_ai::Provider> {
    match p.str_opt("provider") {
        Some(name) => nook_ai::Provider::parse(&name)
            .ok_or_else(|| Error::config(format!("unknown AI provider '{name}'"))),
        None => Ok(session.settings.ai.provider),
    }
}

/// The whole of `ai.generate`, with the client as a parameter so the tests can answer without a
/// network.
pub(crate) fn generate_with(
    session: &mut Session,
    p: &Params,
    client: &dyn nook_ai::Chat,
) -> Result<serde_json::Value> {
    let prompt = p.str_arg("prompt")?.trim().to_string();
    if prompt.is_empty() {
        return Err(Error::config("the prompt is empty"));
    }
    let cfg = session.settings.ai.clone();
    cfg.validate()?;

    // A caller that names a connection gets that connection or an error. Silently drafting
    // MySQL-shaped SQL in the SQLite dialect because a name was mistyped, or because the connection
    // was never opened, is worse than refusing: the dialect is the whole reason to pass `conn`. With
    // no name the session decides, which is the active connection when there is one.
    let conn = match p.conn_id() {
        Some(id) => {
            let index = session.conn_index(&id)?;
            match session.conns.get(index).map(|conn| conn.driver.is_some()) {
                Some(true) => Some(index),
                _ => {
                    return Err(Error::config(format!(
                        "connection '{id}' is not open; open it first"
                    )))
                }
            }
        }
        None => session.resolve_conn(p).ok(),
    };
    let dialect = match conn {
        Some(index) => connection_dialect(session, index),
        None => Dialect::Sqlite,
    };

    // The tables to describe: the ones the caller listed, plus every mention the prompt carries.
    // Deduplication is exact so a name written two ways is looked up twice, which is what keeps a
    // case-mismatched entry from swallowing a mention that would have resolved.
    let mut names: Vec<String> = p.list("tables");
    for mention in nook_ai::mentioned(&prompt) {
        if !names.contains(&mention) {
            names.push(mention);
        }
    }

    let mut contexts: Vec<String> = Vec::new();
    let mut used: Vec<String> = Vec::new();
    match conn {
        Some(index) => {
            for name in &names {
                let reference = TableRef::parse(name);
                match session.table_of(index, reference.schema.as_deref(), &reference.table) {
                    Ok(table) => {
                        contexts.push(nook_ai::table_context(&table));
                        used.push(table.key());
                    }
                    // A mention that matches nothing is a typo, not a reason to refuse: the request
                    // still goes, one table lighter, and the log says which one was dropped and why
                    // — `no such table`, or the catalog has not been read yet.
                    Err(e) => session.log(
                        Level::Warn,
                        format!("ai: '{name}' was not added to the prompt: {e}"),
                    ),
                }
            }
        }
        None if !names.is_empty() => session.log(
            Level::Warn,
            "ai: no connection is open, so table structure could not be read",
        ),
        None => {}
    }

    let key = crate::profiles::ai_key(cfg.provider)?;
    if cfg.provider.needs_key() && key.as_deref().unwrap_or("").trim().is_empty() {
        return Err(Error::config(format!(
            "no API key for {}; run ai.setKey, or set NOOK_AI_KEY for the run",
            cfg.provider.label()
        )));
    }

    let system = nook_ai::system_prompt(dialect, &contexts);
    let answer = client.chat(&cfg, key.as_deref(), &system, &prompt)?;
    let sql = nook_ai::extract_sql(&answer);
    if sql.is_empty() {
        return Err(Error::config("the model answered with nothing"));
    }

    let mut out = json!({
        "sql": sql,
        "provider": cfg.provider.as_str(),
        "model": cfg.model(),
        "endpoint": nook_ai::endpoint(&cfg),
        "dialect": crate::cmd::dialect_name(dialect),
        "tables": used,
    });
    if p.bool_opt("open_tab", false) {
        let index = session.add_tab(TabKind::Query, "AI query", conn);
        if let Some(tab) = session.tabs.get_mut(index) {
            tab.sql.text = sql.clone();
        }
        out["tab"] = json!(index);
    }
    let described = if used.is_empty() {
        "the request alone".to_string()
    } else {
        used.join(", ")
    };
    session.log(
        Level::Success,
        format!("ai: {} drafted SQL from {described}", cfg.model()),
    );
    Ok(out)
}

fn providers(session: &mut Session, _p: &Params) -> Result<serde_json::Value> {
    let cfg = session.settings.ai.clone();
    let mut list = Vec::new();
    for provider in nook_ai::Provider::ALL {
        let key_set = matches!(
            crate::profiles::ai_key(provider),
            Ok(Some(key)) if !key.trim().is_empty()
        );
        list.push(json!({
            "id": provider.as_str(),
            "label": provider.label(),
            "default_base_url": provider.default_base(),
            "default_model": provider.default_model(),
            "needs_key": provider.needs_key(),
            "key_set": key_set,
            "active": provider == cfg.provider,
        }));
    }
    Ok(json!({
        "active": cfg.provider.as_str(),
        "model": cfg.model(),
        "base_url": cfg.base(),
        "endpoint": nook_ai::endpoint(&cfg),
        "max_tokens": cfg.max_tokens,
        "temperature": cfg.temperature,
        "timeout_secs": cfg.timeout_secs,
        "keychain": crate::profiles::keychain_available(),
        "providers": list,
    }))
}

fn set_key(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let provider = provider_from_params(session, p)?;
    let key = p.str_arg("key")?;
    if key.trim().is_empty() {
        return Err(Error::config("the key is empty"));
    }
    if !crate::profiles::keychain_available() {
        return Err(Error::unsupported(crate::profiles::NO_KEYCHAIN_AI));
    }
    crate::profiles::save_ai_key(provider.as_str(), key.trim())?;
    session.log(
        Level::Success,
        format!("{} API key saved in the keychain", provider.label()),
    );
    // The key itself is never echoed back to a caller or into the log.
    Ok(json!({ "provider": provider.as_str(), "key_set": true }))
}

fn clear_key(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let provider = provider_from_params(session, p)?;
    crate::profiles::clear_ai_key(provider.as_str())?;
    session.log(Level::Info, format!("{} API key removed", provider.label()));
    Ok(json!({ "provider": provider.as_str(), "key_set": false }))
}

fn tables(session: &mut Session, p: &Params) -> Result<serde_json::Value> {
    let index = session.resolve_conn(p)?;
    let filter = p
        .str_opt("filter")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let catalog = session.catalog_of(index)?;
    let mut out: Vec<serde_json::Value> = Vec::new();
    for schema in &catalog.schemas {
        for table in &schema.tables {
            let key = table.key();
            if !filter.is_empty() && !key.to_ascii_lowercase().contains(&filter) {
                continue;
            }
            out.push(json!({
                "table": key,
                "kind": table.kind.label(),
                "columns": table.columns.len(),
                "mention": format!("@{key}"),
            }));
        }
    }
    let count = out.len();
    Ok(json!({ "tables": out, "count": count }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{fake_security, scratch_config};
    use std::sync::{Mutex, MutexGuard};

    /// A provider that answers with a fixed reply and keeps the prompt it was handed, so a test can
    /// read what the model would have seen without a network.
    struct Scripted {
        reply: String,
        seen: Mutex<Option<(String, String)>>,
    }

    impl Scripted {
        fn new(reply: &str) -> Self {
            Scripted {
                reply: reply.to_string(),
                seen: Mutex::new(None),
            }
        }

        fn captured(&self) -> (String, String) {
            self.seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
                .unwrap_or_default()
        }

        fn system(&self) -> String {
            self.captured().0
        }
    }

    impl nook_ai::Chat for Scripted {
        fn chat(
            &self,
            _cfg: &nook_ai::AiConfig,
            _key: Option<&str>,
            system: &str,
            user: &str,
        ) -> Result<String> {
            *self
                .seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                Some((system.to_string(), user.to_string()));
            Ok(self.reply.clone())
        }
    }

    /// A live SQLite session holding the two tables the prompt tests mention, pointed at a local
    /// provider so no key is in play.
    fn session_with_tables() -> (MutexGuard<'static, ()>, Session) {
        let guard = scratch_config();
        let mut session = Session::new();
        session
            .run(
                "conn.new",
                json!({"name": "local", "kind": "sqlite", "file": ":memory:"}),
            )
            .expect("new connection");
        session
            .run("conn.open", json!({"conn": "local"}))
            .expect("open connection");
        session
            .run(
                "sql.run",
                json!({"sql": "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL); \
                               CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, total NUMERIC)"}),
            )
            .expect("ddl");
        session
            .run("conn.refresh", json!({"conn": "local"}))
            .expect("refresh catalog");
        session.settings.ai.provider = nook_ai::Provider::Ollama;
        (guard, session)
    }

    fn params(json: serde_json::Value) -> Params {
        Params::from_json(json).expect("params")
    }

    #[test]
    fn the_prompt_carries_the_structure_of_every_mentioned_table() {
        let (_guard, mut session) = session_with_tables();
        let client = Scripted::new("```sql\nSELECT email FROM users;\n```");
        let out = generate_with(
            &mut session,
            &params(json!({
                "conn": "local",
                "prompt": "emails of @users who ordered, joined to @orders"
            })),
            &client,
        )
        .expect("generate");

        assert_eq!(out["sql"], "SELECT email FROM users;");
        assert_eq!(out["provider"], "ollama");
        assert_eq!(out["dialect"], "sqlite");

        let system = client.system();
        assert!(system.contains("TABLE users"), "{system}");
        assert!(system.contains("TABLE orders"), "{system}");
        assert!(system.contains("email TEXT NOT NULL"), "{system}");
        assert!(system.contains("id INTEGER PRIMARY KEY"), "{system}");
        assert!(system.contains("total NUMERIC"), "{system}");
        assert!(system.contains("SQLite"), "{system}");
        assert!(system.contains("no markdown code fence"), "{system}");

        // The user's own words are the user turn, mentions and all.
        assert_eq!(
            client.captured().1,
            "emails of @users who ordered, joined to @orders"
        );

        // Both tables are reported, and no tab was opened because none was asked for.
        assert_eq!(out["tables"].as_array().map(|t| t.len()), Some(2));
        assert!(out.get("tab").is_none());
    }

    #[test]
    fn a_table_named_twice_is_described_once() {
        let (_guard, mut session) = session_with_tables();
        let client = Scripted::new("SELECT 1;");
        let out = generate_with(
            &mut session,
            &params(
                json!({"conn": "local", "prompt": "count rows in @users", "tables": ["users"]}),
            ),
            &client,
        )
        .expect("generate");
        // One request, not two descriptions of the same table.
        assert_eq!(out["tables"].as_array().map(|t| t.len()), Some(1));
        assert_eq!(client.system().matches("TABLE users").count(), 1);
    }

    #[test]
    fn a_name_the_catalog_does_not_hold_leaves_the_mention_that_does() {
        let (_guard, mut session) = session_with_tables();
        let client = Scripted::new("SELECT 1;");
        // Identifiers are matched exactly, the way the rest of the engine matches them, so the
        // named `Users` drops out with a warning while the mention still resolves.
        let out = generate_with(
            &mut session,
            &params(
                json!({"conn": "local", "prompt": "count rows in @users", "tables": ["Users"]}),
            ),
            &client,
        )
        .expect("generate");
        assert_eq!(out["tables"].as_array().map(|t| t.len()), Some(1));
        assert_eq!(client.system().matches("TABLE users").count(), 1);
        assert!(session
            .messages
            .iter()
            .any(|m| m.level == Level::Warn && m.text.contains("'Users'")));
    }

    #[test]
    fn a_table_that_is_not_in_the_catalog_is_a_warning_not_a_failure() {
        let (_guard, mut session) = session_with_tables();
        let client = Scripted::new("SELECT count(*) FROM ghost;");
        let out = generate_with(
            &mut session,
            &params(json!({"conn": "local", "prompt": "count @ghost rows"})),
            &client,
        )
        .expect("a missing table still generates");

        assert_eq!(out["sql"], "SELECT count(*) FROM ghost;");
        assert_eq!(out["tables"].as_array().map(|t| t.len()), Some(0));
        assert!(
            !client.system().contains("ghost"),
            "no structure was invented"
        );
        assert!(session
            .messages
            .iter()
            .any(|m| m.level == Level::Warn && m.text.contains("ghost")));
    }

    #[test]
    fn a_named_connection_that_does_not_exist_or_is_not_open_is_refused() {
        let (_guard, mut session) = session_with_tables();
        let client = Scripted::new("SELECT 1;");

        // A mistyped name used to fall through to "no connection is open" and draft SQLite SQL.
        let missing = generate_with(
            &mut session,
            &params(json!({"conn": "nope", "prompt": "count rows in @users"})),
            &client,
        )
        .expect_err("an unknown connection is an error");
        assert!(missing.to_string().contains("nope"), "{missing}");
        assert_eq!(client.captured().0, "", "nothing was sent");

        // A connection that exists but was never opened has no dialect to read, so the request is
        // refused rather than guessed at in SQLite.
        session.conns[0].driver = None;
        let closed = generate_with(
            &mut session,
            &params(json!({"conn": "local", "prompt": "count rows in @users"})),
            &client,
        )
        .expect_err("a closed connection is an error");
        assert!(closed.to_string().contains("not open"), "{closed}");
        assert_eq!(client.captured().0, "", "nothing was sent");
    }

    #[test]
    fn drafting_without_a_connection_uses_sqlite_and_says_so() {
        let _guard = scratch_config();
        let mut session = Session::new();
        session.settings.ai.provider = nook_ai::Provider::Ollama;
        let client = Scripted::new("SELECT 1;");
        let out = generate_with(
            &mut session,
            &params(json!({"prompt": "one row", "tables": ["users"]})),
            &client,
        )
        .expect("generate");
        assert_eq!(out["dialect"], "sqlite");
        assert!(client.system().contains("SQLite"));
        assert!(session
            .messages
            .iter()
            .any(|m| m.level == Level::Warn && m.text.contains("no connection is open")));
    }

    #[test]
    fn a_hosted_provider_without_a_key_says_how_to_add_one() {
        // A fake keychain that knows a different account, so this provider has no key at all.
        let (_guard, _log) = fake_security("some-other-account", "irrelevant");
        std::env::remove_var("NOOK_AI_KEY");
        let mut session = Session::new();
        session.settings.ai.provider = nook_ai::Provider::OpenAi;
        let client = Scripted::new("SELECT 1;");
        let error = generate_with(&mut session, &params(json!({"prompt": "hi"})), &client)
            .expect_err("no key is a refusal");
        assert!(error.to_string().contains("no API key"), "{error}");
        assert!(error.to_string().contains("ai.setKey"), "{error}");
        // Nothing was sent.
        assert!(client.captured().0.is_empty());
    }

    #[test]
    fn asking_for_a_tab_puts_the_sql_in_the_editor() {
        let (_guard, mut session) = session_with_tables();
        let client = Scripted::new("```sql\nSELECT count(*) FROM users;\n```");
        let out = generate_with(
            &mut session,
            &params(json!({"conn": "local", "prompt": "how many @users", "open_tab": true})),
            &client,
        )
        .expect("generate");
        let index = out["tab"].as_u64().expect("a tab index") as usize;
        assert_eq!(session.tabs[index].kind, TabKind::Query);
        assert_eq!(session.tabs[index].sql.text, "SELECT count(*) FROM users;");
    }

    #[test]
    fn junk_parameters_are_an_error_not_a_request() {
        let (_guard, mut session) = session_with_tables();
        let client = Scripted::new("SELECT 1;");
        // A missing prompt, a blank one, and one of the wrong JSON type.
        assert!(generate_with(&mut session, &Params::none(), &client).is_err());
        assert!(generate_with(&mut session, &params(json!({"prompt": "   "})), &client).is_err());
        assert!(
            generate_with(&mut session, &params(json!({"prompt": {"a": 1}})), &client).is_err()
        );
        // A number would be read as its text, the way every command reads parameters, so it is not
        // an error — but not one of these refusals reached the provider.
        assert!(client.captured().0.is_empty());
    }

    #[test]
    fn an_unknown_provider_name_is_refused() {
        let mut session = Session::new();
        let error = session
            .run(
                "app.setSetting",
                json!({"key": "ai_provider", "value": "not-a-provider"}),
            )
            .expect_err("unknown provider");
        assert!(error.to_string().contains("ai_provider must be"), "{error}");
    }

    #[test]
    fn the_provider_list_reports_whether_a_key_is_set_but_never_the_key() {
        let (_guard, _log) = fake_security("ai.openai", "sk-super-secret-value");
        std::env::remove_var("NOOK_AI_KEY");
        let mut session = Session::new();
        let out = session.run("ai.providers", json!({})).expect("providers");

        let text = serde_json::to_string(&out).expect("serialise");
        assert!(
            !text.contains("sk-super-secret-value"),
            "a key must never be echoed: {text}"
        );

        let list = out["providers"].as_array().expect("a list");
        assert_eq!(list.len(), nook_ai::Provider::ALL.len());
        let find = |id: &str| {
            list.iter()
                .find(|p| p["id"] == id)
                .unwrap_or_else(|| panic!("{id} is listed"))
                .clone()
        };
        assert_eq!(find("openai")["key_set"], true);
        assert_eq!(find("openai")["active"], true);
        assert_eq!(find("openai")["needs_key"], true);
        assert_eq!(find("ollama")["key_set"], false);
        assert_eq!(find("ollama")["needs_key"], false);
        assert_eq!(
            out["endpoint"],
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn saving_a_key_goes_to_the_keychain_and_never_to_the_profile_file() {
        let (_guard, log) = fake_security("ai.openai", "sk-1");
        std::env::remove_var("NOOK_AI_KEY");
        let mut session = Session::new();
        session.settings.ai.provider = nook_ai::Provider::OpenAi;

        if !crate::profiles::keychain_available() {
            let error = session
                .run("ai.setKey", json!({"key": "sk-secret-value"}))
                .expect_err("there is no keychain to write to");
            assert!(error.to_string().contains("NOOK_AI_KEY"), "{error}");
            return;
        }

        let out = session
            .run("ai.setKey", json!({"key": "sk-secret-value"}))
            .expect("save the key");
        assert_eq!(out["key_set"], true);
        assert!(!serde_json::to_string(&out)
            .expect("json")
            .contains("sk-secret-value"));

        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(calls.contains("add-generic-password"), "{calls}");
        assert!(calls.contains("ai.openai"), "{calls}");
        // The store file the app owns holds no secret.
        let profile = std::fs::read_to_string(crate::profiles::profiles_path()).unwrap_or_default();
        assert!(!profile.contains("sk-secret-value"), "{profile}");
    }

    #[test]
    fn the_mention_candidates_come_from_the_live_catalog() {
        let (_guard, mut session) = session_with_tables();
        let out = session
            .run("ai.tables", json!({"conn": "local", "filter": "user"}))
            .expect("tables");
        let names: Vec<String> = out["tables"]
            .as_array()
            .expect("a list")
            .iter()
            .filter_map(|t| t["table"].as_str().map(str::to_string))
            .collect();
        assert!(names.contains(&"users".to_string()), "{names:?}");
        assert!(!names.contains(&"orders".to_string()), "{names:?}");
        assert_eq!(out["tables"][0]["mention"], "@users");
        assert_eq!(out["count"], names.len());
    }
}
