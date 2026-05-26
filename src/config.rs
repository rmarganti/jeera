use crate::client::JiraAuth;
use crate::client::JiraClientConfig;
use serde::Deserialize;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use url::Url;

// ----------------------------------------------------------------
// Public config model
// ----------------------------------------------------------------

pub struct Settings {
    pub base_url: Url,
    pub auth: AuthSettings,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthSettings {
    Basic { email: String, api_token: String },
    Bearer { token: String },
}

// ----------------------------------------------------------------
// Errors
// ----------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not determine the config directory; set JEERA_CONFIG or HOME")]
    ConfigDirectoryNotFound,

    #[error(
        "config file not found at {path}\n\nCreate it with:\n{{\n  \"base_url\": \"https://your-domain.atlassian.net\",\n  \"auth\": {{\n    \"type\": \"basic\",\n    \"email\": \"you@example.com\",\n    \"api_token\": \"<jira-api-token>\"\n  }}\n}}\n\nOr set JEERA_CONFIG=/path/to/settings.json"
    )]
    MissingFile { path: PathBuf },

    #[error("failed to read {path}: {source}", path = .path.display())]
    ReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "failed to parse {path}: {source}\n\nSupported auth types: basic, bearer",
        path = .path.display()
    )]
    ParseFailed {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("invalid base_url {value:?}: {reason}")]
    InvalidBaseUrl { value: String, reason: &'static str },

    #[error("invalid auth config: {reason}")]
    InvalidAuth { reason: &'static str },
}

// ----------------------------------------------------------------
// Secret-safe debug formatting
// ----------------------------------------------------------------

impl fmt::Debug for Settings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Settings")
            .field("base_url", &self.base_url)
            .field("auth", &self.auth)
            .finish()
    }
}

impl fmt::Debug for AuthSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Basic { email, .. } => f
                .debug_struct("Basic")
                .field("email", email)
                .field("api_token", &"<redacted>")
                .finish(),
            Self::Bearer { .. } => f
                .debug_struct("Bearer")
                .field("token", &"<redacted>")
                .finish(),
        }
    }
}

// ----------------------------------------------------------------
// Config loading and conversion
// ----------------------------------------------------------------

impl Settings {
    pub fn load() -> Result<Self, ConfigError> {
        let path = settings_path()?;
        Self::load_from_path(&path)
    }

    fn load_from_path(path: &Path) -> Result<Self, ConfigError> {
        let raw = fs::read_to_string(path).map_err(|source| match source.kind() {
            std::io::ErrorKind::NotFound => ConfigError::MissingFile {
                path: path.to_path_buf(),
            },
            _ => ConfigError::ReadFailed {
                path: path.to_path_buf(),
                source,
            },
        })?;

        let settings: RawSettings =
            serde_json::from_str(&raw).map_err(|source| ConfigError::ParseFailed {
                path: path.to_path_buf(),
                source,
            })?;

        settings.validate()
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

// ----------------------------------------------------------------
// Raw config model (pre-validation)
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawSettings {
    base_url: String,
    auth: AuthSettings,
}

impl RawSettings {
    fn validate(self) -> Result<Settings, ConfigError> {
        let base_url = validate_base_url(&self.base_url)?;
        validate_auth(&self.auth)?;

        Ok(Settings {
            base_url,
            auth: self.auth,
        })
    }
}

// ----------------------------------------------------------------
// Validation
// ----------------------------------------------------------------

fn validate_base_url(value: &str) -> Result<Url, ConfigError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::InvalidBaseUrl {
            value: value.to_string(),
            reason: "must not be empty",
        });
    }

    let mut url = Url::parse(trimmed).map_err(|_| ConfigError::InvalidBaseUrl {
        value: value.to_string(),
        reason: "must be an absolute http or https URL",
    })?;

    match url.scheme() {
        "http" | "https" => {}
        _ => {
            return Err(ConfigError::InvalidBaseUrl {
                value: value.to_string(),
                reason: "must use http or https",
            });
        }
    }

    if url.host_str().is_none() {
        return Err(ConfigError::InvalidBaseUrl {
            value: value.to_string(),
            reason: "must include a host",
        });
    }

    if url.path().ends_with('/') {
        let normalized_path = url.path().trim_end_matches('/').to_string();
        url.set_path(if normalized_path.is_empty() {
            "/"
        } else {
            &normalized_path
        });
    }

    Ok(url)
}

fn validate_auth(auth: &AuthSettings) -> Result<(), ConfigError> {
    match auth {
        AuthSettings::Basic { email, api_token } => {
            if email.trim().is_empty() {
                return Err(ConfigError::InvalidAuth {
                    reason: "basic auth requires a non-empty email",
                });
            }

            if api_token.trim().is_empty() {
                return Err(ConfigError::InvalidAuth {
                    reason: "basic auth requires a non-empty api_token",
                });
            }
        }
        AuthSettings::Bearer { token } => {
            if token.trim().is_empty() {
                return Err(ConfigError::InvalidAuth {
                    reason: "bearer auth requires a non-empty token",
                });
            }
        }
    }

    Ok(())
}

// ----------------------------------------------------------------
// Config path resolution
// ----------------------------------------------------------------

fn settings_path() -> Result<PathBuf, ConfigError> {
    settings_path_with(|key| env::var_os(key))
}

fn settings_path_with<F>(getenv: F) -> Result<PathBuf, ConfigError>
where
    F: Fn(&str) -> Option<std::ffi::OsString>,
{
    if let Some(path) = getenv("JEERA_CONFIG") {
        return Ok(PathBuf::from(path));
    }

    if let Some(path) = getenv("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path).join("jeera").join("settings.json"));
    }

    if let Some(path) = getenv("HOME") {
        return Ok(PathBuf::from(path)
            .join(".config")
            .join("jeera")
            .join("settings.json"));
    }

    Err(ConfigError::ConfigDirectoryNotFound)
}

// ----------------------------------------------------------------
// Tests
// ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn prefers_jeera_config_override() {
        let path = settings_path_with(|key| match key {
            "JEERA_CONFIG" => Some("/tmp/custom-jeera.json".into()),
            "XDG_CONFIG_HOME" => Some("/tmp/xdg".into()),
            "HOME" => Some("/tmp/home".into()),
            _ => None,
        })
        .unwrap();

        assert_eq!(path, PathBuf::from("/tmp/custom-jeera.json"));
    }

    #[test]
    fn falls_back_to_xdg_then_home() {
        let xdg_path = settings_path_with(|key| match key {
            "XDG_CONFIG_HOME" => Some("/tmp/xdg".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(xdg_path, PathBuf::from("/tmp/xdg/jeera/settings.json"));

        let home_path = settings_path_with(|key| match key {
            "HOME" => Some("/tmp/home".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(
            home_path,
            PathBuf::from("/tmp/home/.config/jeera/settings.json")
        );
    }

    #[test]
    fn validates_and_normalizes_base_url() {
        let settings = RawSettings {
            base_url: " https://example.atlassian.net/ ".to_string(),
            auth: AuthSettings::Bearer {
                token: "secret".to_string(),
            },
        }
        .validate()
        .unwrap();

        assert_eq!(settings.base_url.as_str(), "https://example.atlassian.net/");
    }

    #[test]
    fn rejects_invalid_auth() {
        let error = RawSettings {
            base_url: "https://example.atlassian.net".to_string(),
            auth: AuthSettings::Basic {
                email: " ".to_string(),
                api_token: "secret".to_string(),
            },
        }
        .validate()
        .unwrap_err();

        assert!(matches!(error, ConfigError::InvalidAuth { .. }));
    }

    #[test]
    fn redacts_secrets_in_debug_output() {
        let settings = RawSettings {
            base_url: "https://example.atlassian.net".to_string(),
            auth: AuthSettings::Basic {
                email: "you@example.com".to_string(),
                api_token: "secret-token".to_string(),
            },
        }
        .validate()
        .unwrap();

        let debug = format!("{settings:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret-token"));
    }

    #[test]
    fn missing_file_error_includes_setup_guidance() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("jeera-missing-{unique}.json"));

        let error = Settings::load_from_path(&path).unwrap_err();
        let rendered = error.to_string();

        assert!(matches!(error, ConfigError::MissingFile { .. }));
        assert!(rendered.contains("Create it with:"));
        assert!(rendered.contains("JEERA_CONFIG"));
    }
}
