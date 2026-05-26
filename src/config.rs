use crate::client::JiraAuth;
use crate::client::JiraClientConfig;
use serde::Deserialize;
use std::env;
use std::fmt;
use std::fs;
use std::path::PathBuf;

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

#[derive(Debug)]
pub enum ConfigError {
    HomeDirectoryNotFound,
    ReadFailed { path: PathBuf, message: String },
    ParseFailed { path: PathBuf, message: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::HomeDirectoryNotFound => {
                write!(f, "could not determine the home directory")
            }
            ConfigError::ReadFailed { path, message } => {
                write!(f, "failed to read {}: {message}", path.display())
            }
            ConfigError::ParseFailed { path, message } => {
                write!(f, "failed to parse {}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Settings {
    pub fn load() -> Result<Self, ConfigError> {
        let path = settings_path()?;
        let raw = fs::read_to_string(&path).map_err(|error| ConfigError::ReadFailed {
            path: path.clone(),
            message: error.to_string(),
        })?;

        serde_json::from_str(&raw).map_err(|error| ConfigError::ParseFailed {
            path,
            message: error.to_string(),
        })
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
