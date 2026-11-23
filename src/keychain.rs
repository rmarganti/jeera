//! Jira token storage in the macOS login Keychain.
//!
//! Tokens are stored as generic passwords under the `jeera` service, keyed by the
//! normalized Jira `base_url`. Entries created by `jeera auth set-token` trust only the
//! jeera binary that created them, so other tools (including `security
//! find-generic-password`) trigger a macOS approval prompt before they can read the token.
//!
//! On other platforms lookups find nothing and writes fail with
//! [`KeychainError::Unsupported`]; use `JEERA_API_TOKEN` there instead.

use thiserror::Error;

pub const SERVICE: &str = "jeera";

#[derive(Debug, Error)]
pub enum KeychainError {
    #[error("access to the jeera Keychain entry was denied")]
    Denied,

    #[error(
        "the Keychain could not ask for approval in this session (it may be locked, or there is no GUI, e.g. over SSH); set JEERA_API_TOKEN instead"
    )]
    InteractionNotAllowed,

    #[cfg_attr(target_os = "macos", allow(dead_code))]
    #[error("the macOS Keychain is not available on this platform; set JEERA_API_TOKEN instead")]
    Unsupported,

    #[error("the token stored in the Keychain is not valid UTF-8")]
    InvalidUtf8,

    #[error("Keychain error: {message}")]
    Other { message: String },
}

/// Look up the stored token for `account`. Returns `Ok(None)` when no entry exists.
pub fn get_token(account: &str) -> Result<Option<String>, KeychainError> {
    imp::get_token(account)
}

/// Create or replace the stored token for `account`.
pub fn set_token(account: &str, token: &str) -> Result<(), KeychainError> {
    imp::set_token(account, token)
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{KeychainError, SERVICE};
    use security_framework::base::Error as SecError;
    use security_framework::passwords::{get_generic_password, set_generic_password};

    // OSStatus codes from Security.framework's SecBase.h.
    const ERR_SEC_USER_CANCELED: i32 = -128;
    const ERR_SEC_AUTH_FAILED: i32 = -25293;
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
    const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;

    pub fn get_token(account: &str) -> Result<Option<String>, KeychainError> {
        match get_generic_password(SERVICE, account) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| KeychainError::InvalidUtf8),
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(error) => Err(map_error(error)),
        }
    }

    pub fn set_token(account: &str, token: &str) -> Result<(), KeychainError> {
        set_generic_password(SERVICE, account, token.as_bytes()).map_err(map_error)
    }

    fn map_error(error: SecError) -> KeychainError {
        match error.code() {
            ERR_SEC_USER_CANCELED | ERR_SEC_AUTH_FAILED => KeychainError::Denied,
            ERR_SEC_INTERACTION_NOT_ALLOWED => KeychainError::InteractionNotAllowed,
            _ => KeychainError::Other {
                message: error.to_string(),
            },
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::KeychainError;

    pub fn get_token(_account: &str) -> Result<Option<String>, KeychainError> {
        Ok(None)
    }

    pub fn set_token(_account: &str, _token: &str) -> Result<(), KeychainError> {
        Err(KeychainError::Unsupported)
    }
}
