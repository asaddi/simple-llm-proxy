#![warn(clippy::pedantic)]

use std::collections::HashMap;

use anyhow::{Context, Result};
use noyalib::from_str;
use tracing::{Level, event};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Config {
    pub connect_timeout: Option<u64>,
    pub read_timeout: Option<u64>,
    pub total_timeout: Option<u64>,

    providers: Vec<ProviderConfig>,
    models: Vec<ModelConfig>,

    require_auth: Option<Vec<AuthToken>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct ProviderConfig {
    name: String,
    base_url: String,
    api_key: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct ModelConfig {
    name: String,
    provider: String,
    model: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct AuthToken {
    id: String,
    token: String,
}

#[derive(Debug, serde::Serialize)]
pub struct ProcessedConfig {
    provider_map: HashMap<String, ProviderConfig>,
    model_map: HashMap<String, ModelConfig>,
    models: Vec<String>,

    auth_tokens: HashMap<String, String>,
}

#[derive(Debug, serde::Serialize)]
pub struct ModelTarget {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
}

impl Config {
    pub fn load(config_file: &str) -> Result<Config> {
        let config_str = std::fs::read_to_string(config_file)
            .with_context(|| format!("failed to open {config_file}"))?;
        from_str(config_str.as_str()).with_context(|| format!("failed to parse {config_file}"))
    }

    pub fn process_config(self) -> ProcessedConfig {
        let mut provider_map = HashMap::new();
        for p in &self.providers {
            let resolved_provider = ProviderConfig {
                name: p.name.clone(),
                base_url: p.base_url.trim_end_matches('/').to_string(),
                api_key: p.api_key.as_ref().map(|k| resolve_api_key(k)),
            };
            if provider_map
                .insert(resolved_provider.name.clone(), resolved_provider)
                .is_some()
            {
                event!(
                    Level::WARN,
                    "duplicate provider '{}'; later one wins",
                    p.name
                );
            }
        }
        let mut model_map = HashMap::new();
        let mut models = Vec::new();
        for m in &self.models {
            if model_map.insert(m.name.clone(), m.clone()).is_some() {
                event!(Level::WARN, "duplicate model '{}'; later one wins", m.name);
            } else {
                models.push(m.name.clone());
            }
        }

        let mut auth_tokens = HashMap::new();
        for a in &self.require_auth.unwrap_or_default() {
            let token = resolve_api_key(a.token.as_str());
            if auth_tokens.insert(token, a.id.clone()).is_some() {
                event!(Level::WARN, "duplicate auth token with id '{}'", a.id);
            }
        }

        ProcessedConfig {
            provider_map,
            model_map,
            models,
            auth_tokens,
        }
    }
}

#[test]
#[should_panic(expected = "Environment variable 'DUMMY_API_KEY'")]
fn test_missing_env() {
    let config = Config::load("test/config-basic.yaml").unwrap();
    temp_env::with_var_unset("DUMMY_API_KEY", || {
        let _ = config.process_config();
    });
}

#[test]
fn test_basic_env() {
    let config = Config::load("test/config-basic.yaml").unwrap();
    temp_env::with_var("DUMMY_API_KEY", Some("sk-54321"), || {
        let processed = config.process_config();
        insta::assert_yaml_snapshot!(processed, {
            ".provider_map" => insta::sorted_redaction(),
            ".model_map" => insta::sorted_redaction(),
            ".models" => insta::sorted_redaction(),
        });
    });
}

#[test]
fn test_auth_tokens() {
    let config = Config::load("test/config-auth.yaml").unwrap();
    temp_env::with_vars(
        [
            ("DUMMY_API_KEY", Some("sk-54321")),
            ("MY_ENV_TOKEN", Some("my-98765")),
        ],
        || {
            let processed = config.process_config();
            insta::assert_yaml_snapshot!(processed, {
                ".provider_map" => insta::sorted_redaction(),
                ".model_map" => insta::sorted_redaction(),
                ".models" => insta::sorted_redaction(),
                ".auth_tokens" => insta::sorted_redaction(),
            });
        },
    );
}

fn resolve_api_key(key: &str) -> String {
    let lower = key.to_lowercase();
    if lower.starts_with("env:") {
        let env_var = &key[4..];
        std::env::var(env_var)
            .unwrap_or_else(|_| panic!("Environment variable '{env_var}' not found"))
        // FIXME don't panic
    } else {
        key.to_string()
    }
}

impl ProcessedConfig {
    pub fn get_models(&self) -> Vec<String> {
        self.models.clone()
    }

    pub fn get_target(&self, model: &str) -> Option<ModelTarget> {
        if let Some(model_config) = self.model_map.get(model) {
            // TODO Yes, if the provider doesn't exist, that is an error condition.
            // But maaaaybe we shouldn't crash.
            let provider_config = self
                .provider_map
                .get(model_config.provider.as_str())
                .unwrap();
            Some(ModelTarget {
                base_url: provider_config.base_url.clone(),
                api_key: provider_config.api_key.clone(),
                model: model_config.model.clone(),
            })
        } else {
            None
        }
    }

    pub fn auth_check(&self, token: Option<&str>) -> bool {
        if self.auth_tokens.is_empty() {
            // No tokens defined, everyone's allowed
            true
        } else {
            token.is_some_and(|tok| {
                if let Some(id) = self.auth_tokens.get(tok) {
                    event!(Level::DEBUG, "authorized token id {id}");
                    true
                } else {
                    event!(Level::TRACE, "not authorized");
                    false
                }
            })
        }
    }
}

#[test]
fn test_get_models() {
    let config = Config::load("test/config-basic.yaml").unwrap();
    temp_env::with_var("DUMMY_API_KEY", Some("sk-54321"), || {
        let processed = config.process_config();
        insta::assert_yaml_snapshot!(&processed.get_models(), {
            "." => insta::sorted_redaction(),
        });
    });
}

#[test]
fn test_get_target_found() {
    let config = Config::load("test/config-basic.yaml").unwrap();
    temp_env::with_var("DUMMY_API_KEY", Some("sk-54321"), || {
        let processed = config.process_config();
        insta::assert_yaml_snapshot!(&processed.get_target("local/model").unwrap());
        insta::assert_yaml_snapshot!(&processed.get_target("remote/model").unwrap());
    });
}

#[test]
fn test_get_target_not_found() {
    let config = Config::load("test/config-basic.yaml").unwrap();
    temp_env::with_var("DUMMY_API_KEY", Some("sk-54321"), || {
        let processed = config.process_config();
        assert!(processed.get_target("somerandommodel").is_none());
    });
}

#[test]
fn test_auth_check_not_required() {
    let config = Config::load("test/config-basic.yaml").unwrap();
    temp_env::with_var("DUMMY_API_KEY", Some("sk-54321"), || {
        let processed = config.process_config();
        assert!(processed.auth_check(None));
        assert!(processed.auth_check(Some("my-invalid-key")));
        assert!(processed.auth_check(Some("my-12345")));
        assert!(processed.auth_check(Some("my-98765")));
    });
}

#[test]
fn test_auth_check_required() {
    let config = Config::load("test/config-auth.yaml").unwrap();
    temp_env::with_vars(
        [
            ("DUMMY_API_KEY", Some("sk-54321")),
            ("MY_ENV_TOKEN", Some("my-98765")),
        ],
        || {
            let processed = config.process_config();
            assert!(!processed.auth_check(None));
            assert!(!processed.auth_check(Some("my-invalid-key")));
            assert!(processed.auth_check(Some("my-12345")));
            assert!(processed.auth_check(Some("my-98765")));
        },
    );
}
