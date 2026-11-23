use crate::client::JiraAuth;
use crate::client::JiraClientConfig;
use crate::keychain::{self, KeychainError};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;
use url::Url;

// ----------------------------------------------------------------
// Public config model
// ----------------------------------------------------------------

pub struct Settings {
    pub base_url: Url,
    pub auth: AuthSettings,
    pub http_timeout: Duration,
    pub mutations_enabled: bool,
    pub default_board_id: Option<u64>,
    pub searches: BTreeMap<String, SearchProfileSettings>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct SearchProfileSettings {
    pub board: Option<String>,
    pub jql: Option<String>,
    pub project: Option<String>,
    pub assignee: Option<String>,
    #[serde(default)]
    pub unassigned: bool,
    pub reporter: Option<String>,
    #[serde(default)]
    pub status: Vec<String>,
    pub status_category: Option<String>,
    #[serde(default, alias = "issue_type")]
    pub issue_type: Vec<String>,
    #[serde(default)]
    pub component: Vec<String>,
    #[serde(default)]
    pub label: Vec<String>,
    pub text: Option<String>,
    #[serde(default)]
    pub open: bool,
    pub limit: Option<u32>,
    pub sort: Option<String>,
    #[serde(default)]
    pub asc: bool,
    #[serde(default)]
    pub desc: bool,
}

/// Resolved auth credentials. Tokens come from `JEERA_API_TOKEN`, the macOS Keychain, or
/// (deprecated) plaintext in the config file — see [`TokenSource`].
pub enum AuthSettings {
    Basic { email: String, api_token: String },
    Bearer { token: String },
}

/// Where the resolved token came from, in lookup order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    Env,
    Keychain,
    ConfigFile,
}

/// Env var that overrides any stored token.
pub const TOKEN_ENV_VAR: &str = "JEERA_API_TOKEN";

/// What `jeera auth set-token` needs from the config without resolving a token.
pub struct AuthTarget {
    pub keychain_account: String,
    pub has_plaintext_token: bool,
}

// ----------------------------------------------------------------
// Errors
// ----------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not determine the config directory; set JEERA_CONFIG or HOME")]
    ConfigDirectoryNotFound,

    #[error(
        "config file not found at {path}\n\nCreate it with:\n{{\n  \"base_url\": \"https://your-domain.atlassian.net\",\n  \"http_timeout_seconds\": 30,\n  \"auth\": {{\n    \"type\": \"basic\",\n    \"email\": \"you@example.com\"\n  }}\n}}\n\nThen store your token in the macOS Keychain with:\n  jeera auth set-token\n\nOr set JEERA_CONFIG=/path/to/settings.json"
    )]
    MissingFile { path: PathBuf },

    #[error(
        "no Jira token found for {account}\n\nStore one in the macOS Keychain with:\n  jeera auth set-token\n\nOr set {TOKEN_ENV_VAR}"
    )]
    MissingToken { account: String },

    #[error("while reading the Jira token from the Keychain: {source}")]
    Keychain { source: KeychainError },

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

    #[error("invalid http_timeout_seconds {value}: {reason}")]
    InvalidHttpTimeout { value: u64, reason: &'static str },

    #[error("invalid search profile {name:?}: {reason}")]
    InvalidSearchProfile { name: String, reason: &'static str },
}

// ----------------------------------------------------------------
// Secret-safe debug formatting
// ----------------------------------------------------------------

impl fmt::Debug for Settings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Settings")
            .field("base_url", &self.base_url)
            .field("http_timeout_seconds", &self.http_timeout.as_secs())
            .field("mutations_enabled", &self.mutations_enabled)
            .field("default_board_id", &self.default_board_id)
            .field("searches", &self.searches)
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
        let raw = load_raw(&path)?;
        let has_plaintext_token = raw.auth.plaintext_token().is_some();

        let (settings, source) =
            raw.validate_with(|key| env::var(key).ok(), keychain::get_token)?;

        match (source, has_plaintext_token) {
            (TokenSource::ConfigFile, _) => eprintln!(
                "warning: reading the Jira token from plaintext in {} is deprecated; run `jeera auth set-token` and remove it from the config",
                path.display()
            ),
            (_, true) => eprintln!(
                "warning: {} still contains a plaintext Jira token that is no longer used; remove it",
                path.display()
            ),
            _ => {}
        }

        Ok(settings)
    }

    pub fn into_jira_client_config(self) -> JiraClientConfig {
        let auth = match self.auth {
            AuthSettings::Basic { email, api_token } => JiraAuth::Basic { email, api_token },
            AuthSettings::Bearer { token } => JiraAuth::Bearer { token },
        };

        JiraClientConfig {
            base_url: self.base_url,
            auth,
            timeout: self.http_timeout,
            default_board_id: self.default_board_id,
            searches: self.searches,
        }
    }
}

/// Read just enough config to manage the stored token. Never touches the Keychain.
pub fn load_auth_target() -> Result<AuthTarget, ConfigError> {
    let raw = load_raw(&settings_path()?)?;
    let base_url = validate_base_url(&raw.base_url)?;

    Ok(AuthTarget {
        keychain_account: keychain_account(&base_url),
        has_plaintext_token: raw.auth.plaintext_token().is_some(),
    })
}

fn load_raw(path: &Path) -> Result<RawSettings, ConfigError> {
    let raw = fs::read_to_string(path).map_err(|source| match source.kind() {
        std::io::ErrorKind::NotFound => ConfigError::MissingFile {
            path: path.to_path_buf(),
        },
        _ => ConfigError::ReadFailed {
            path: path.to_path_buf(),
            source,
        },
    })?;

    serde_json::from_str(&raw).map_err(|source| ConfigError::ParseFailed {
        path: path.to_path_buf(),
        source,
    })
}

/// Keychain entries are keyed by the normalized base URL so several Jira sites can coexist.
fn keychain_account(base_url: &Url) -> String {
    base_url.as_str().to_string()
}

// ----------------------------------------------------------------
// Raw config model (pre-validation)
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawSettings {
    base_url: String,
    #[serde(default = "default_http_timeout_seconds")]
    http_timeout_seconds: u64,
    #[serde(default)]
    mutations_enabled: bool,
    #[serde(default)]
    default_board_id: Option<u64>,
    #[serde(default)]
    searches: BTreeMap<String, SearchProfileSettings>,
    auth: RawAuthSettings,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RawAuthSettings {
    Basic {
        email: String,
        #[serde(default)]
        api_token: Option<String>,
    },
    Bearer {
        #[serde(default)]
        token: Option<String>,
    },
}

impl fmt::Debug for RawAuthSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Basic { email, .. } => f.debug_struct("Basic").field("email", email).finish(),
            Self::Bearer { .. } => f.debug_struct("Bearer").finish_non_exhaustive(),
        }
    }
}

impl RawAuthSettings {
    /// The deprecated plaintext token from the config file, if a non-blank one is set.
    fn plaintext_token(&self) -> Option<&str> {
        let token = match self {
            Self::Basic { api_token, .. } => api_token.as_deref(),
            Self::Bearer { token } => token.as_deref(),
        };
        token.filter(|token| !token.trim().is_empty())
    }
}

impl RawSettings {
    fn validate_with<E, K>(
        self,
        getenv: E,
        keychain_lookup: K,
    ) -> Result<(Settings, TokenSource), ConfigError>
    where
        E: Fn(&str) -> Option<String>,
        K: Fn(&str) -> Result<Option<String>, KeychainError>,
    {
        let base_url = validate_base_url(&self.base_url)?;
        let http_timeout = validate_http_timeout(self.http_timeout_seconds)?;
        validate_searches(&self.searches)?;

        if let RawAuthSettings::Basic { email, .. } = &self.auth
            && email.trim().is_empty()
        {
            return Err(ConfigError::InvalidAuth {
                reason: "basic auth requires a non-empty email",
            });
        }

        let account = keychain_account(&base_url);
        let (token, source) = resolve_token(
            self.auth.plaintext_token(),
            &account,
            getenv,
            keychain_lookup,
        )?;

        let auth = match self.auth {
            RawAuthSettings::Basic { email, .. } => AuthSettings::Basic {
                email,
                api_token: token,
            },
            RawAuthSettings::Bearer { .. } => AuthSettings::Bearer { token },
        };

        Ok((
            Settings {
                base_url,
                auth,
                http_timeout,
                mutations_enabled: self.mutations_enabled,
                default_board_id: self.default_board_id,
                searches: self.searches,
            },
            source,
        ))
    }
}

/// Resolve the token: `JEERA_API_TOKEN`, then the Keychain, then plaintext config.
///
/// A missing Keychain entry falls through silently. Any other Keychain failure (denied,
/// no GUI to prompt) is an error rather than a silent fallback, so a denial is never
/// papered over by a stale plaintext token.
fn resolve_token<E, K>(
    config_token: Option<&str>,
    account: &str,
    getenv: E,
    keychain_lookup: K,
) -> Result<(String, TokenSource), ConfigError>
where
    E: Fn(&str) -> Option<String>,
    K: Fn(&str) -> Result<Option<String>, KeychainError>,
{
    let non_blank = |token: String| (!token.trim().is_empty()).then_some(token);

    if let Some(token) = getenv(TOKEN_ENV_VAR).and_then(non_blank) {
        return Ok((token, TokenSource::Env));
    }

    let stored = keychain_lookup(account).map_err(|source| ConfigError::Keychain { source })?;
    if let Some(token) = stored.and_then(non_blank) {
        return Ok((token, TokenSource::Keychain));
    }

    if let Some(token) = config_token {
        return Ok((token.to_string(), TokenSource::ConfigFile));
    }

    Err(ConfigError::MissingToken {
        account: account.to_string(),
    })
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

    if !url.path().ends_with('/') {
        let mut normalized_path = url.path().to_string();
        normalized_path.push('/');
        url.set_path(&normalized_path);
    }

    Ok(url)
}

fn default_http_timeout_seconds() -> u64 {
    30
}

fn validate_http_timeout(value: u64) -> Result<Duration, ConfigError> {
    if value == 0 {
        return Err(ConfigError::InvalidHttpTimeout {
            value,
            reason: "must be greater than zero",
        });
    }

    Ok(Duration::from_secs(value))
}

fn validate_searches(
    searches: &BTreeMap<String, SearchProfileSettings>,
) -> Result<(), ConfigError> {
    for (name, profile) in searches {
        if name.trim().is_empty() {
            return Err(ConfigError::InvalidSearchProfile {
                name: name.clone(),
                reason: "profile names must not be empty",
            });
        }

        if profile.asc && profile.desc {
            return Err(ConfigError::InvalidSearchProfile {
                name: name.clone(),
                reason: "cannot set both asc and desc",
            });
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

    fn raw_settings(
        base_url: &str,
        http_timeout_seconds: u64,
        auth: RawAuthSettings,
    ) -> RawSettings {
        RawSettings {
            base_url: base_url.to_string(),
            http_timeout_seconds,
            mutations_enabled: false,
            default_board_id: None,
            searches: BTreeMap::new(),
            auth,
        }
    }

    fn bearer(token: &str) -> RawAuthSettings {
        RawAuthSettings::Bearer {
            token: Some(token.to_string()),
        }
    }

    fn basic(email: &str, api_token: &str) -> RawAuthSettings {
        RawAuthSettings::Basic {
            email: email.to_string(),
            api_token: Some(api_token.to_string()),
        }
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn empty_keychain(_: &str) -> Result<Option<String>, KeychainError> {
        Ok(None)
    }

    impl RawSettings {
        /// Validate with no env override and an empty Keychain, so the config token is used.
        fn validate_without_stores(self) -> Result<Settings, ConfigError> {
            self.validate_with(no_env, empty_keychain)
                .map(|(settings, _)| settings)
        }
    }

    fn token_of(settings: &Settings) -> &str {
        match &settings.auth {
            AuthSettings::Basic { api_token, .. } => api_token,
            AuthSettings::Bearer { token } => token,
        }
    }

    #[test]
    fn env_token_overrides_keychain_and_config() {
        let (settings, source) = raw_settings("https://example.atlassian.net", 30, bearer("file"))
            .validate_with(
                |key| (key == TOKEN_ENV_VAR).then(|| "from-env".to_string()),
                |_| panic!("keychain should not be consulted when the env var is set"),
            )
            .unwrap();

        assert_eq!(source, TokenSource::Env);
        assert_eq!(token_of(&settings), "from-env");
    }

    #[test]
    fn blank_env_token_is_ignored() {
        let (_, source) = raw_settings("https://example.atlassian.net", 30, bearer("file"))
            .validate_with(|_| Some("  ".to_string()), empty_keychain)
            .unwrap();

        assert_eq!(source, TokenSource::ConfigFile);
    }

    #[test]
    fn keychain_token_is_looked_up_by_normalized_base_url() {
        let auth = RawAuthSettings::Basic {
            email: "you@example.com".to_string(),
            api_token: None,
        };
        let (settings, source) = raw_settings(" https://example.atlassian.net ", 30, auth)
            .validate_with(no_env, |account| {
                assert_eq!(account, "https://example.atlassian.net/");
                Ok(Some("from-keychain".to_string()))
            })
            .unwrap();

        assert_eq!(source, TokenSource::Keychain);
        assert_eq!(token_of(&settings), "from-keychain");
    }

    #[test]
    fn keychain_token_wins_over_plaintext_config() {
        let (settings, source) = raw_settings("https://example.atlassian.net", 30, bearer("file"))
            .validate_with(no_env, |_| Ok(Some("from-keychain".to_string())))
            .unwrap();

        assert_eq!(source, TokenSource::Keychain);
        assert_eq!(token_of(&settings), "from-keychain");
    }

    #[test]
    fn falls_back_to_plaintext_config_token() {
        let (settings, source) = raw_settings("https://example.atlassian.net", 30, bearer("file"))
            .validate_with(no_env, empty_keychain)
            .unwrap();

        assert_eq!(source, TokenSource::ConfigFile);
        assert_eq!(token_of(&settings), "file");
    }

    #[test]
    fn missing_token_everywhere_explains_how_to_store_one() {
        let auth = RawAuthSettings::Bearer {
            token: Some(" ".to_string()),
        };
        let error = raw_settings("https://example.atlassian.net", 30, auth)
            .validate_without_stores()
            .unwrap_err();
        let rendered = error.to_string();

        assert!(matches!(error, ConfigError::MissingToken { .. }));
        assert!(rendered.contains("jeera auth set-token"));
        assert!(rendered.contains(TOKEN_ENV_VAR));
    }

    #[test]
    fn keychain_failure_is_an_error_not_a_silent_fallback() {
        let error = raw_settings("https://example.atlassian.net", 30, bearer("file"))
            .validate_with(no_env, |_| Err(KeychainError::Denied))
            .unwrap_err();

        assert!(matches!(
            error,
            ConfigError::Keychain {
                source: KeychainError::Denied
            }
        ));
    }

    #[test]
    fn parses_auth_without_a_plaintext_token() {
        let raw: RawSettings = serde_json::from_str(
            r#"{"base_url":"https://example.atlassian.net","auth":{"type":"basic","email":"you@example.com"}}"#,
        )
        .unwrap();

        assert!(raw.auth.plaintext_token().is_none());
    }

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
        let settings = raw_settings(
            " https://example.atlassian.net/ ",
            default_http_timeout_seconds(),
            bearer("secret"),
        )
        .validate_without_stores()
        .unwrap();

        assert_eq!(settings.base_url.as_str(), "https://example.atlassian.net/");
    }

    #[test]
    fn preserves_base_url_path_prefix_for_joining() {
        let settings = raw_settings(
            "https://example.atlassian.net/jira",
            default_http_timeout_seconds(),
            bearer("secret"),
        )
        .validate_without_stores()
        .unwrap();

        assert_eq!(
            settings.base_url.as_str(),
            "https://example.atlassian.net/jira/"
        );
        assert_eq!(
            settings
                .base_url
                .join("rest/api/3/search/jql")
                .unwrap()
                .as_str(),
            "https://example.atlassian.net/jira/rest/api/3/search/jql"
        );
    }

    #[test]
    fn rejects_invalid_auth() {
        let error = raw_settings(
            "https://example.atlassian.net",
            default_http_timeout_seconds(),
            basic(" ", "secret"),
        )
        .validate_without_stores()
        .unwrap_err();

        assert!(matches!(error, ConfigError::InvalidAuth { .. }));
    }

    #[test]
    fn rejects_invalid_base_url() {
        let error = raw_settings(
            "jira-relative-path",
            default_http_timeout_seconds(),
            bearer("secret"),
        )
        .validate_without_stores()
        .unwrap_err();

        assert!(matches!(error, ConfigError::InvalidBaseUrl { .. }));
    }

    #[test]
    fn redacts_secrets_in_debug_output() {
        let settings = raw_settings(
            "https://example.atlassian.net",
            default_http_timeout_seconds(),
            basic("you@example.com", "secret-token"),
        )
        .validate_without_stores()
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

        let error = load_raw(&path).unwrap_err();
        let rendered = error.to_string();

        assert!(matches!(error, ConfigError::MissingFile { .. }));
        assert!(rendered.contains("Create it with:"));
        assert!(rendered.contains("http_timeout_seconds"));
        assert!(rendered.contains("JEERA_CONFIG"));
    }

    #[test]
    fn parse_failure_names_supported_auth_types() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("jeera-invalid-{unique}.json"));
        fs::write(
            &path,
            "{\"base_url\":\"https://example.atlassian.net\",\"auth\":{\"type\":\"oauth\"}}",
        )
        .unwrap();

        let error = load_raw(&path).unwrap_err();
        let rendered = error.to_string();

        assert!(matches!(error, ConfigError::ParseFailed { .. }));
        assert!(rendered.contains("Supported auth types: basic, bearer"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn defaults_http_timeout_to_thirty_seconds() {
        let settings = raw_settings(
            "https://example.atlassian.net",
            default_http_timeout_seconds(),
            bearer("secret"),
        )
        .validate_without_stores()
        .unwrap();

        assert_eq!(settings.http_timeout, Duration::from_secs(30));
    }

    #[test]
    fn mutations_are_disabled_by_default() {
        let raw = r#"{
            "base_url": "https://example.atlassian.net",
            "auth": { "type": "bearer", "token": "secret" }
        }"#;

        let settings: RawSettings = serde_json::from_str(raw).unwrap();
        let settings = settings.validate_without_stores().unwrap();

        assert!(!settings.mutations_enabled);
    }

    #[test]
    fn mutations_can_be_explicitly_enabled() {
        let raw = r#"{
            "base_url": "https://example.atlassian.net",
            "mutations_enabled": true,
            "auth": { "type": "bearer", "token": "secret" }
        }"#;

        let settings: RawSettings = serde_json::from_str(raw).unwrap();
        let settings = settings.validate_without_stores().unwrap();

        assert!(settings.mutations_enabled);
    }

    #[test]
    fn accepts_default_board_id() {
        let mut settings = raw_settings(
            "https://example.atlassian.net",
            default_http_timeout_seconds(),
            bearer("secret"),
        );
        settings.default_board_id = Some(215);

        let settings = settings.validate_without_stores().unwrap();

        assert_eq!(settings.default_board_id, Some(215));
    }

    #[test]
    fn accepts_search_profiles() {
        let mut settings = raw_settings(
            "https://example.atlassian.net",
            default_http_timeout_seconds(),
            bearer("secret"),
        );
        settings.searches.insert(
            "backend".to_string(),
            SearchProfileSettings {
                project: Some("SAMPLE".to_string()),
                component: vec!["BACKEND".to_string()],
                limit: Some(25),
                ..Default::default()
            },
        );

        let settings = settings.validate_without_stores().unwrap();

        assert_eq!(settings.searches.len(), 1);
        let profile = settings.searches.get("backend").unwrap();
        assert_eq!(profile.project.as_deref(), Some("SAMPLE"));
        assert_eq!(profile.component, vec!["BACKEND"]);
        assert_eq!(profile.limit, Some(25));
    }

    #[test]
    fn rejects_conflicting_profile_sort_directions() {
        let mut settings = raw_settings(
            "https://example.atlassian.net",
            default_http_timeout_seconds(),
            bearer("secret"),
        );
        settings.searches.insert(
            "backend".to_string(),
            SearchProfileSettings {
                asc: true,
                desc: true,
                ..Default::default()
            },
        );

        let error = settings.validate_without_stores().unwrap_err();

        assert!(matches!(error, ConfigError::InvalidSearchProfile { .. }));
    }

    #[test]
    fn accepts_custom_http_timeout() {
        let settings = raw_settings("https://example.atlassian.net", 7, bearer("secret"))
            .validate_without_stores()
            .unwrap();

        assert_eq!(settings.http_timeout, Duration::from_secs(7));
    }

    #[test]
    fn rejects_zero_http_timeout() {
        let error = raw_settings("https://example.atlassian.net", 0, bearer("secret"))
            .validate_without_stores()
            .unwrap_err();

        assert!(matches!(error, ConfigError::InvalidHttpTimeout { .. }));
    }
}
