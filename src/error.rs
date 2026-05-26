use crate::client::types::JiraError;
use crate::config::ConfigError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("while loading config: {source}")]
    LoadConfig { source: ConfigError },
    #[error("while executing search: {source}")]
    ExecuteSearch { source: JiraError },
    #[error("while writing output: {source}")]
    RenderOutput { source: std::io::Error },
}
