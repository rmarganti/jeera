use crate::client::types::JiraError;
use crate::config::ConfigError;
use serde_json::Error as SerdeJsonError;
use std::num::ParseIntError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("while loading config: {source}")]
    LoadConfig { source: ConfigError },

    #[error("invalid boards: {reason}")]
    InvalidBoards { reason: String },

    #[error("invalid search: {reason}")]
    InvalidSearch { reason: String },

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

    #[error("while writing output: {source}")]
    RenderOutput { source: std::io::Error },

    #[error("while encoding json output: {source}")]
    EncodeJsonOutput { source: SerdeJsonError },
}
