use crate::client::JiraAuth;
use crate::client::JiraClientConfig;
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub base_url: String,
    pub auth: AuthSettings,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthSettings {
    Basic { email: String, api_token: String },
    Bearer { token: String },
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not determine the home directory")]
    HomeDirectoryNotFound,
    #[error("failed to read {path}: {source}", path = .path.display())]
    ReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}", path = .path.display())]
    ParseFailed {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

impl Settings {
    pub fn load() -> Result<Self, ConfigError> {
        let path = settings_path()?;
        let raw = fs::read_to_string(&path).map_err(|source| ConfigError::ReadFailed {
            path: path.clone(),
            source,
        })?;

        serde_json::from_str(&raw).map_err(|source| ConfigError::ParseFailed { path, source })
    }

    pub fn into_jira_client_config(self) -> JiraClientConfig {
        let auth = match self.auth {
            AuthSettings::Basic { email, api_token } => JiraAuth::Basic { email, api_token },
            AuthSettings::Bearer { token } => JiraAuth::Bearer { token },
        };

        JiraClientConfig {
            base_url: self.base_url,
            auth,
        }
    }
}

fn settings_path() -> Result<PathBuf, ConfigError> {
    let home = env::var_os("HOME").ok_or(ConfigError::HomeDirectoryNotFound)?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("jeera")
        .join("settings.json"))
}
