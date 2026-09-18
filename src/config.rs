#![warn(clippy::pedantic)]

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use noyalib::from_str;
use regex::Regex;
use serde_json::Value;
use tracing::{Level, event};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Config {
    pub connect_timeout: Option<u64>,
    pub read_timeout: Option<u64>,
    pub total_timeout: Option<u64>,

    providers: Vec<ProviderConfig>,
    models: Vec<ModelConfig>,
    remaps: Option<Vec<RemapConfig>>,

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
struct RemapConfig {
    prefix: String,
    provider: String,
    filters: Option<Vec<String>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct AuthToken {
    id: String,
    token: String,
}

#[derive(Debug, serde::Serialize)]
pub struct ProcessedConfig {
    pub remaps: Vec<ProcessedRemap>,
    auth_tokens: HashMap<String, String>,
    config_models: ModelMap,

    // TODO does the following need to be atomic?
    all_models: ModelMap,
}

#[derive(Debug, serde::Serialize)]
pub struct ProcessedRemap {
    prefix: String,
    provider: String,
    base_url: String,
    api_key: Option<String>,
    #[serde(skip)]
    filters: Vec<Regex>,

    // TODO does the following need to be atomic?
    models: ModelMap,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ModelMap {
    models: Vec<String>,
    model_map: HashMap<String, ModelTarget>,
}

#[derive(Debug, Clone, serde::Serialize)]
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

        let mut remaps = Vec::new();
        for m in &self.remaps.unwrap_or_default() {
            if let Some(provider_config) = provider_map.get(&m.provider) {
                let regexps =
                    m.filters.clone().map_or_default(|filters| {
                        Vec::from_iter(filters.iter().map(|re| {
                            Regex::new(re).unwrap_or_else(|_| panic!("bad regex: '{re}'"))
                        }))
                    });
                let processed_remap = ProcessedRemap {
                    prefix: m.prefix.clone(),
                    provider: provider_config.name.clone(),
                    base_url: provider_config.base_url.clone(),
                    api_key: provider_config.api_key.clone(),
                    filters: regexps,
                    models: ModelMap::new(),
                };
                remaps.push(processed_remap);
            } else {
                event!(
                    Level::WARN,
                    "remap prefix '{}' references unknown provider '{}'; ignoring",
                    m.prefix,
                    m.provider
                );
            }
        }

        let mut auth_tokens = HashMap::new();
        for a in &self.require_auth.unwrap_or_default() {
            let token = resolve_api_key(a.token.as_str());
            if auth_tokens.insert(token, a.id.clone()).is_some() {
                event!(Level::WARN, "duplicate auth token with id '{}'", a.id);
            }
        }

        let mut config_models = ModelMap::new();
        for m in &self.models {
            if let Some(provider_config) = provider_map.get(&m.provider) {
                if config_models.insert(
                    &m.name,
                    ModelTarget {
                        base_url: provider_config.base_url.clone(),
                        api_key: provider_config.api_key.clone(),
                        model: m.model.clone(),
                    },
                ) {
                    event!(Level::WARN, "duplicate model '{}'; later one wins", m.name);
                }
            } else {
                event!(
                    Level::WARN,
                    "model '{}' references unknown provider '{}'; ignoring",
                    m.name,
                    m.provider
                );
            }
        }

        // For now, it's just the configured models.
        let all_models = config_models.clone();

        ProcessedConfig {
            remaps,
            auth_tokens,
            config_models,
            all_models,
        }
    }
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
        self.all_models.models()
    }

    pub fn get_target(&self, model: &str) -> Option<ModelTarget> {
        self.all_models.get(model)
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

    #[allow(dead_code)]
    pub fn update_models(&mut self) {
        let mut all_models = ModelMap::new();

        // First, the models from prefix remaps, in order.
        for remap in &self.remaps {
            for remap_model in &remap.models.models {
                let model_target = remap.models.get(remap_model).unwrap();
                all_models.insert(remap_model, model_target);
            }
        }

        // Then the virtual models from the config.
        for model in &self.config_models.models {
            let model_target = self.config_models.get(model).unwrap();
            all_models.insert(model, model_target);
        }

        self.all_models = all_models;
    }
}

impl ProcessedRemap {
    fn is_model_selected(&self, model: &str) -> bool {
        if self.filters.is_empty() {
            true
        } else {
            self.filters.iter().any(|re| re.is_match(model))
        }
    }

    #[allow(dead_code)]
    pub fn populate(&mut self, source: &Value) -> Result<()> {
        let models = source
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| anyhow!("missing data array"))?;

        let mut new_models = ModelMap::new();
        for m in models {
            let model = m
                .get("id")
                .and_then(|id| id.as_str())
                .ok_or_else(|| anyhow!("missing model id"))?;
            if self.is_model_selected(model) {
                let mut remapped_name = String::from(&self.prefix);
                remapped_name.push_str(model);

                new_models.insert(
                    &remapped_name,
                    ModelTarget {
                        base_url: self.base_url.clone(),
                        api_key: self.api_key.clone(),
                        model: model.to_owned(),
                    },
                );
            }
        }

        self.models = new_models;
        Ok(())
    }
}

impl ModelMap {
    fn new() -> ModelMap {
        Self {
            models: Vec::new(),
            model_map: HashMap::new(),
        }
    }

    fn insert(&mut self, model: &str, model_target: ModelTarget) -> bool {
        if self
            .model_map
            .insert(model.to_owned(), model_target)
            .is_none()
        {
            self.models.push(model.to_owned());
            false
        } else {
            true
        }
    }

    fn get(&self, model: &str) -> Option<ModelTarget> {
        self.model_map.get(model).cloned()
    }

    fn models(&self) -> Vec<String> {
        self.models.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::Config;
    use insta::{assert_ron_snapshot, sorted_redaction};

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
            assert_ron_snapshot!(processed, {
                ".config_models.models" => sorted_redaction(),
                ".config_models.model_map" => sorted_redaction(),
                ".all_models.models" => sorted_redaction(),
                ".all_models.model_map" => sorted_redaction(),
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
                assert_ron_snapshot!(processed, {
                    ".auth_tokens" => sorted_redaction(),
                    ".config_models.models" => sorted_redaction(),
                    ".config_models.model_map" => sorted_redaction(),
                    ".all_models.models" => sorted_redaction(),
                    ".all_models.model_map" => sorted_redaction(),
                });
            },
        );
    }

    #[test]
    fn test_get_models() {
        let config = Config::load("test/config-basic.yaml").unwrap();
        temp_env::with_var("DUMMY_API_KEY", Some("sk-54321"), || {
            let processed = config.process_config();
            assert_ron_snapshot!(&processed.get_models(), {
                "." => sorted_redaction(),
            });
        });
    }

    #[test]
    fn test_get_target_found() {
        let config = Config::load("test/config-basic.yaml").unwrap();
        temp_env::with_var("DUMMY_API_KEY", Some("sk-54321"), || {
            let processed = config.process_config();
            assert_ron_snapshot!(&processed.get_target("local/model").unwrap());
            assert_ron_snapshot!(&processed.get_target("remote/model").unwrap());
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

    #[test]
    fn test_remaps() {
        let config = Config::load("test/config-remap.yaml").unwrap();
        let processed = config.process_config();
        assert_ron_snapshot!(processed, {
            ".remaps" => sorted_redaction(),
            ".config_models.models" => sorted_redaction(),
            ".config_models.model_map" => sorted_redaction(),
            ".all_models.models" => sorted_redaction(),
            ".all_models.model_map" => sorted_redaction(),
        });
    }

    #[test]
    fn test_is_model_selected() {
        let config = Config::load("test/config-remap.yaml").unwrap();
        let processed = config.process_config();

        // The "remote/" remap has a filter.
        let remap = &processed
            .remaps
            .iter()
            .find(|r| r.prefix == "remote/")
            .unwrap();
        assert!(remap.is_model_selected("mymodel1234"));
        assert!(!remap.is_model_selected("blahmymodel1234"));
        assert!(!remap.is_model_selected("someothermodel"));

        // The "remote2/" remap doesn't.
        let remap = &processed
            .remaps
            .iter()
            .find(|r| r.prefix == "remote2/")
            .unwrap();
        assert!(remap.is_model_selected("mymodel1234"));
        assert!(remap.is_model_selected("blahmymodel1234"));
        assert!(remap.is_model_selected("someothermodel"));
    }

    #[test]
    fn test_basic_remaps_no_match() {
        let config = Config::load("test/config-remap.yaml").unwrap();
        let mut processed = config.process_config();
        let remap1 = &mut processed.remaps[0];
        remap1
            .populate(&serde_json::json!({
                "data": [
                {
                    "id": "blahmodel",
                    "owned_by": "meeee",
                }
            ]}))
            .unwrap();
        let remap2 = &mut processed.remaps[1];
        remap2
            .populate(&serde_json::json!({
                "data": [
                {
                    "id": "nofilter",
                    "owned_by": "meeee",
                }
            ]}))
            .unwrap();
        processed.update_models();
        assert_ron_snapshot!(processed, {
            ".remaps" => sorted_redaction(),
            ".config_models.models" => sorted_redaction(),
            ".config_models.model_map" => sorted_redaction(),
            ".all_models.models" => sorted_redaction(),
            ".all_models.model_map" => sorted_redaction(),
        });
    }

    #[test]
    fn test_basic_remaps_match() {
        let config = Config::load("test/config-remap.yaml").unwrap();
        let mut processed = config.process_config();
        let remap1 = &mut processed.remaps[0];
        remap1
            .populate(&serde_json::json!({
                "data": [
                    {
                        "id": "blahmodel",
                        "owned_by": "meeee",
                    },
                    {
                        "id": "mymodel123",
                        "owned_by": "meeee",
                    }
            ]}))
            .unwrap();
        let remap2 = &mut processed.remaps[1];
        remap2
            .populate(&serde_json::json!({
                "data": [
                {
                    "id": "nofilter",
                    "owned_by": "meeee",
                }
            ]}))
            .unwrap();
        processed.update_models();
        assert_ron_snapshot!(processed, {
            ".remaps" => sorted_redaction(),
            ".config_models.models" => sorted_redaction(),
            ".config_models.model_map" => sorted_redaction(),
            ".all_models.models" => sorted_redaction(),
            ".all_models.model_map" => sorted_redaction(),
        });
    }
}
