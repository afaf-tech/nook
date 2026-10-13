#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! L1: drafting SQL with an AI provider.
//!
//! The crate is small on purpose. A provider is a row of data (`provider.rs`), the request shape is
//! one function per wire format (`client.rs`), and turning a sentence plus a table's structure into
//! a prompt is the third file. The transport is the system `curl` — see `client.rs` for the
//! measurement that chose it and for where an API key is allowed to live.
//!
//! Nothing here reads the filesystem, the keychain or the database: the engine owns those and hands
//! this crate a prompt and a config.

mod client;
mod prompt;
mod provider;

pub use client::{
    body, chat_via_curl, curl_bin, endpoint, error_message, headers, parse_response, Chat, CurlChat,
};
pub use prompt::{extract_sql, mentioned, system_prompt, table_context};
pub use provider::{Provider, Wire};

use nook_core::{Error, Result};
use serde::{Deserialize, Serialize};

fn default_provider() -> Provider {
    Provider::OpenAi
}

fn default_max_tokens() -> u32 {
    1024
}

fn default_temperature() -> f32 {
    0.0
}

fn default_timeout_secs() -> u64 {
    60
}

/// The AI settings a user can change, persisted with the rest of the preferences.
///
/// `model` and `base_url` are empty by default and fall back to the provider's own values, so
/// switching provider cannot leave a stale model behind. The API key is deliberately **not** here:
/// it lives in the keychain (`engine::profiles`), so `profiles.json` never holds a secret.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default = "default_provider")]
    pub provider: Provider,
    /// Empty means the provider's own default model.
    #[serde(default)]
    pub model: String,
    /// Empty means the provider's own endpoint. Set it for a proxy, a gateway or a local server.
    #[serde(default)]
    pub base_url: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

impl Default for AiConfig {
    fn default() -> Self {
        AiConfig {
            provider: Provider::OpenAi,
            model: String::new(),
            base_url: String::new(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            timeout_secs: default_timeout_secs(),
        }
    }
}

impl AiConfig {
    /// The model a request is sent with: the chosen one, or the provider's default.
    pub fn model(&self) -> String {
        if self.model.trim().is_empty() {
            self.provider.default_model().to_string()
        } else {
            self.model.trim().to_string()
        }
    }

    /// The API root a request is sent to: the chosen one, or the provider's default.
    pub fn base(&self) -> String {
        if self.base_url.trim().is_empty() {
            self.provider.default_base().to_string()
        } else {
            self.base_url.trim().to_string()
        }
    }

    /// Refuse before a request is built, with the sentence that says what to do about it.
    pub fn validate(&self) -> Result<()> {
        if self.model().trim().is_empty() {
            return Err(Error::config(format!(
                "set a model for {} — a {} server serves whatever it was started with",
                self.provider.label(),
                self.provider.label()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_untouched_config_uses_the_provider_defaults() {
        let cfg = AiConfig::default();
        assert_eq!(cfg.provider, Provider::OpenAi);
        assert_eq!(cfg.model(), "gpt-4o-mini");
        assert_eq!(cfg.base(), "https://api.openai.com/v1");
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn switching_provider_moves_the_defaults_with_it() {
        let cfg = AiConfig {
            provider: Provider::Ollama,
            ..AiConfig::default()
        };
        assert_eq!(cfg.model(), "llama3.1");
        assert_eq!(cfg.base(), "http://localhost:11434/v1");
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn a_chosen_model_or_url_wins_over_the_default() {
        let cfg = AiConfig {
            model: " gpt-4o ".into(),
            base_url: " https://proxy/v1 ".into(),
            ..AiConfig::default()
        };
        assert_eq!(cfg.model(), "gpt-4o");
        assert_eq!(cfg.base(), "https://proxy/v1");
    }

    #[test]
    fn vllm_without_a_model_is_refused_with_a_reason() {
        let cfg = AiConfig {
            provider: Provider::Vllm,
            ..AiConfig::default()
        };
        let error = cfg.validate().expect_err("vLLM has no default model");
        assert!(error.to_string().contains("set a model"), "{error}");
    }

    #[test]
    fn settings_survive_a_round_trip_and_older_blobs_still_load() {
        let cfg = AiConfig {
            provider: Provider::Kimi,
            model: "kimi-k2".into(),
            base_url: "https://proxy.internal/v1".into(),
            max_tokens: 2048,
            temperature: 0.3,
            timeout_secs: 120,
        };
        let json = serde_json::to_string(&cfg).expect("serialise");
        let back: AiConfig = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back, cfg);

        // A blob written before a field existed: the defaults fill in, nothing fails.
        let older: AiConfig =
            serde_json::from_str(r#"{"provider":"anthropic","model":"claude-3-5-haiku-latest"}"#)
                .expect("an older blob loads");
        assert_eq!(older.provider, Provider::Anthropic);
        assert_eq!(older.max_tokens, 1024);
        assert_eq!(older.timeout_secs, 60);
        assert!(older.base_url.is_empty());

        // And an empty blob is simply the default.
        let empty: AiConfig = serde_json::from_str("{}").expect("empty blob");
        assert_eq!(empty, AiConfig::default());
    }
}
