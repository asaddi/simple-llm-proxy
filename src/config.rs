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

#[derive(Debug)]
pub struct ProcessedConfig {
    provider_map: HashMap<String, ProviderConfig>,
    model_map: HashMap<String, ModelConfig>,
}

#[derive(Debug)]
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
        for m in &self.models {
            if model_map.insert(m.name.clone(), m.clone()).is_some() {
                event!(Level::WARN, "duplicate model '{}'; later one wins", m.name);
            }
        }

        ProcessedConfig {
            provider_map,
            model_map,
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
    pub fn get_models(&self) -> Vec<&String> {
        Vec::from_iter(self.model_map.keys())
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
}
