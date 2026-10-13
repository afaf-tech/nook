//! The providers Nook can draft SQL with, and the shape each one expects.
//!
//! Seven names, two wire formats: everything except Anthropic speaks the OpenAI
//! `/chat/completions` dialect, so one request builder and one response reader cover six of them.
//! A provider is data here, not a type — adding one is a row in this file, not a new module.

use serde::{Deserialize, Serialize};

/// Which wire format a provider speaks. `client.rs` has one request builder per variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    /// `POST {base}/chat/completions`, `Authorization: Bearer …`.
    OpenAi,
    /// `POST {base}/v1/messages`, `x-api-key` plus a pinned `anthropic-version`.
    Anthropic,
}

/// The providers this build knows how to talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    OpenAi,
    Anthropic,
    DeepSeek,
    Kimi,
    Glm,
    Ollama,
    Vllm,
}

/// Reading goes through [`Provider::parse`], so a settings file that names a provider the way a
/// person would — `moonshot`, `z.ai`, `Claude` — loads as the provider it means instead of failing
/// and taking the rest of the blob's settings down with it. Writing stays the canonical id.
impl<'de> Deserialize<'de> for Provider {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Provider, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let name = String::deserialize(deserializer)?;
        Provider::parse(&name).ok_or_else(|| {
            <D::Error as serde::de::Error>::custom(format!("unknown AI provider '{name}'"))
        })
    }
}

impl Provider {
    /// Every provider, in the order the settings surface lists them.
    pub const ALL: [Provider; 7] = [
        Provider::OpenAi,
        Provider::Anthropic,
        Provider::DeepSeek,
        Provider::Kimi,
        Provider::Glm,
        Provider::Ollama,
        Provider::Vllm,
    ];

    /// The provider behind a name a user typed. Punctuation is ignored, so `open_ai`, `z.ai` and
    /// `OpenAI` all land, and the vendor's own product name (`claude`, `moonshot`, `zhipu`) works
    /// as well as the name this file uses.
    pub fn parse(name: &str) -> Option<Provider> {
        let key: String = name
            .trim()
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        match key.as_str() {
            "openai" | "gpt" | "chatgpt" => Some(Provider::OpenAi),
            "anthropic" | "claude" => Some(Provider::Anthropic),
            "deepseek" => Some(Provider::DeepSeek),
            "kimi" | "moonshot" | "kimichat" => Some(Provider::Kimi),
            "glm" | "zhipu" | "zai" | "bigmodel" | "chatglm" => Some(Provider::Glm),
            "ollama" => Some(Provider::Ollama),
            "vllm" => Some(Provider::Vllm),
            _ => None,
        }
    }

    /// The stable id: also the settings value and the keychain account suffix.
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::OpenAi => "openai",
            Provider::Anthropic => "anthropic",
            Provider::DeepSeek => "deepseek",
            Provider::Kimi => "kimi",
            Provider::Glm => "glm",
            Provider::Ollama => "ollama",
            Provider::Vllm => "vllm",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::OpenAi => "OpenAI",
            Provider::Anthropic => "Anthropic",
            Provider::DeepSeek => "DeepSeek",
            Provider::Kimi => "Kimi (Moonshot)",
            Provider::Glm => "GLM (Zhipu)",
            Provider::Ollama => "Ollama",
            Provider::Vllm => "vLLM",
        }
    }

    pub fn wire(self) -> Wire {
        match self {
            Provider::Anthropic => Wire::Anthropic,
            _ => Wire::OpenAi,
        }
    }

    /// The endpoint a request is sent to when the user has not overridden it. The version segment
    /// is included, because that is the part people get wrong when they set a URL by hand.
    pub fn default_base(self) -> &'static str {
        match self {
            Provider::OpenAi => "https://api.openai.com/v1",
            Provider::Anthropic => "https://api.anthropic.com/v1",
            Provider::DeepSeek => "https://api.deepseek.com/v1",
            Provider::Kimi => "https://api.moonshot.cn/v1",
            Provider::Glm => "https://open.bigmodel.cn/api/paas/v4",
            Provider::Ollama => "http://localhost:11434/v1",
            Provider::Vllm => "http://localhost:8000/v1",
        }
    }

    /// The version segment a base URL must end with before the path is appended.
    pub fn version(self) -> &'static str {
        match self {
            Provider::Glm => "/v4",
            _ => "/v1",
        }
    }

    /// The path appended to the base URL.
    pub fn path(self) -> &'static str {
        match self {
            Provider::Anthropic => "/messages",
            _ => "/chat/completions",
        }
    }

    /// The model used when the user has not chosen one.
    pub fn default_model(self) -> &'static str {
        match self {
            Provider::OpenAi => "gpt-4o-mini",
            Provider::Anthropic => "claude-3-5-sonnet-latest",
            Provider::DeepSeek => "deepseek-chat",
            Provider::Kimi => "moonshot-v1-8k",
            Provider::Glm => "glm-4-flash",
            Provider::Ollama => "llama3.1",
            // A vLLM server serves whatever it was started with; there is no safe guess.
            Provider::Vllm => "",
        }
    }

    /// Whether a key is required. A local server takes none, and sending an empty
    /// `Authorization` header is worse than sending none at all.
    pub fn needs_key(self) -> bool {
        !matches!(self, Provider::Ollama | Provider::Vllm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_vendor_product_names_both_resolve() {
        assert_eq!(Provider::parse("OpenAI"), Some(Provider::OpenAi));
        assert_eq!(Provider::parse("open_ai"), Some(Provider::OpenAi));
        assert_eq!(Provider::parse(" claude "), Some(Provider::Anthropic));
        assert_eq!(Provider::parse("moonshot"), Some(Provider::Kimi));
        assert_eq!(Provider::parse("z.ai"), Some(Provider::Glm));
        assert_eq!(Provider::parse("vLLM"), Some(Provider::Vllm));
        assert_eq!(Provider::parse("nope"), None);
    }

    #[test]
    fn every_id_round_trips_and_is_unique() {
        let mut ids: Vec<&str> = Provider::ALL.iter().map(|p| p.as_str()).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(before, ids.len(), "two providers share an id");
        for provider in Provider::ALL {
            assert_eq!(Provider::parse(provider.as_str()), Some(provider));
            assert!(!provider.label().trim().is_empty());
            assert!(provider.default_base().starts_with("http"));
        }
    }

    #[test]
    fn local_servers_need_no_key() {
        assert!(!Provider::Ollama.needs_key());
        assert!(!Provider::Vllm.needs_key());
        assert!(Provider::OpenAi.needs_key());
        assert!(Provider::Anthropic.needs_key());
    }

    #[test]
    fn serde_uses_the_id() {
        let json = serde_json::to_string(&Provider::Vllm).expect("serialise");
        assert_eq!(json, "\"vllm\"");
        let back: Provider = serde_json::from_str("\"openai\"").expect("deserialise");
        assert_eq!(back, Provider::OpenAi);
    }

    #[test]
    fn serde_reads_a_name_a_person_would_write() {
        // A settings file is hand-editable, so reading is as forgiving as the settings command.
        for (name, expected) in [
            ("\"moonshot\"", Provider::Kimi),
            ("\"z.ai\"", Provider::Glm),
            ("\"Claude\"", Provider::Anthropic),
        ] {
            let read: Provider = serde_json::from_str(name).expect("deserialise");
            assert_eq!(read, expected, "{name}");
        }
        assert!(serde_json::from_str::<Provider>("\"opemai\"").is_err());
    }
}
