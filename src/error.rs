use crate::client::types::JiraError;
use crate::config::ConfigError;
use crate::keychain::KeychainError;
use serde_json::Error as SerdeJsonError;
use std::num::ParseIntError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error(
        "mutation commands are disabled; a human must manually set `mutations_enabled` to `true` in the jeera config before proceeding"
    )]
    MutationsDisabled,

    #[error("while loading config: {source}")]
    LoadConfig { source: ConfigError },

    #[error("while reading the token to store: {source}")]
    ReadToken { source: std::io::Error },

    #[error("no token provided")]
    EmptyToken,

    #[error("while storing the token in the Keychain: {source}")]
    StoreToken { source: KeychainError },

    #[error("invalid boards: {reason}")]
    InvalidBoards { reason: String },

    #[error("invalid search: {reason}")]
    InvalidSearch { reason: String },

    #[error("invalid metadata request: {reason}")]
    InvalidMetadata { reason: String },

    #[error("invalid create request: {reason}")]
    InvalidCreate { reason: String },

    #[error("invalid update request: {reason}")]
    InvalidUpdate { reason: String },

    #[error("while preparing board {board_id} for search: {source}")]
    PrepareBoardSearch { board_id: u64, source: JiraError },

    #[error(
        "invalid Jira board configuration for board {board_id}: filter id {filter_id:?} is not numeric: {source}"
    )]
    InvalidBoardFilterId {
        board_id: u64,
        filter_id: String,
        #[source]
        source: ParseIntError,
    },

    #[error("while executing boards: {source}")]
    ExecuteBoards { source: JiraError },

    #[error("while executing search: {source}")]
    ExecuteSearch { source: JiraError },

    #[error("while executing show: {source}")]
    ExecuteShow { source: JiraError },

    #[error("while executing metadata request: {source}")]
    ExecuteMetadata { source: JiraError },

    #[error("while executing create: {source}")]
    ExecuteCreate { source: JiraError },

    #[error("while executing update: {source}")]
    ExecuteUpdate { source: JiraError },

    #[error("while reading input: {source}")]
    ReadInput { source: std::io::Error },

    #[error("while writing output: {source}")]
    RenderOutput { source: std::io::Error },

    #[error("while encoding json output: {source}")]
    EncodeJsonOutput { source: SerdeJsonError },
}
