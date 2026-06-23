use crate::client::types::JiraError;
use crate::config::ConfigError;
use serde_json::Error as SerdeJsonError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("while loading config: {source}")]
    LoadConfig { source: ConfigError },

    #[error("while executing search: {source}")]
    ExecuteSearch { source: JiraError },

    #[error("while executing show: {source}")]
    ExecuteShow { source: JiraError },

    #[error("while writing output: {source}")]
    RenderOutput { source: std::io::Error },

    #[error("while encoding json output: {source}")]
    EncodeJsonOutput { source: SerdeJsonError },
}
