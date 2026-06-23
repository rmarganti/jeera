use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;
use url::ParseError;

// ----------------------------------------------------------------
// Common
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "F: Deserialize<'de>"))]
pub struct IssueResponse<F = Value> {
    #[allow(dead_code)]
    pub id: String,
    pub key: String,
    #[allow(dead_code)]
    #[serde(rename = "self")]
    pub self_link: String,
    pub fields: F,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct JiraErrorResponse {
    #[serde(default)]
    pub error_messages: Vec<String>,

    #[serde(default)]
    pub errors: BTreeMap<String, String>,

    #[serde(default)]
    pub message: Option<String>,

    #[allow(dead_code)]
    #[serde(default)]
    pub status: Option<u16>,

    #[allow(dead_code)]
    #[serde(default)]
    pub http_status_code: Option<u16>,
}

impl JiraErrorResponse {
    pub fn summary(&self) -> Option<String> {
        let mut parts = Vec::new();

        if let Some(message) = &self.message {
            let message = message.trim();
            if !message.is_empty() {
                parts.push(message.to_string());
            }
        }

        if !self.error_messages.is_empty() {
            parts.push(self.error_messages.join("; "));
        }

        if !self.errors.is_empty() {
            parts.push(
                self.errors
                    .iter()
                    .map(|(field, message)| format!("{field}: {message}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join("; "))
        }
    }
}

#[derive(Debug, Error)]
pub enum JiraError {
    #[error("while building Jira URL: {source}")]
    BuildUrl {
        #[source]
        source: ParseError,
    },
    #[error("while building HTTP request: {source}")]
    BuildRequest {
        #[source]
        source: ureq::http::Error,
    },
    #[error("while encoding Jira request body: {source}")]
    EncodeRequestBody {
        #[source]
        source: serde_json::Error,
    },
    #[error("while sending HTTP request: {source}")]
    Transport {
        #[source]
        source: ureq::Error,
    },
    #[error("jira returned HTTP {status} for {endpoint}: {message}")]
    HttpStatus {
        status: u16,
        endpoint: String,
        message: String,
    },
    #[error("while reading Jira response body: {source}")]
    ReadResponseBody {
        #[source]
        source: ureq::Error,
    },
    #[error("while decoding Jira response: {source}")]
    DecodeResponse {
        #[source]
        source: serde_json::Error,
    },
}

// ----------------------------------------------------------------
// Get Issue
// ----------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct GetIssueRequest {
    pub issue_id_or_key: String,
    pub fields: Vec<String>,
    pub expand: Vec<String>,
}

pub type GetIssueResponse<F = Value> = IssueResponse<F>;

// ----------------------------------------------------------------
// Search Issues
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchIssuesRequest {
    pub jql: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub fields: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub expand: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub properties: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields_by_keys: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub reconcile_issues: Vec<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", bound(deserialize = "F: Deserialize<'de>"))]
pub struct SearchIssuesResponse<F = Value> {
    pub is_last: bool,
    #[serde(default)]
    pub issues: Vec<IssueResponse<F>>,
    #[allow(dead_code)]
    #[serde(default)]
    pub names: BTreeMap<String, String>,
    pub next_page_token: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    pub schema: BTreeMap<String, Value>,
}

// ----------------------------------------------------------------
// List Boards
// ----------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ListBoardsRequest {
    pub project_key_or_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListBoardsResponse {
    #[allow(dead_code)]
    #[serde(default)]
    pub max_results: u32,
    #[allow(dead_code)]
    #[serde(default)]
    pub start_at: u32,
    #[allow(dead_code)]
    #[serde(default)]
    pub is_last: bool,
    #[serde(default)]
    pub values: Vec<BoardResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardResponse {
    pub id: u64,
    pub name: String,
    #[serde(rename = "type")]
    pub board_type: String,
    pub location: Option<BoardLocationResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardLocationResponse {
    #[allow(dead_code)]
    pub project_id: Option<u64>,
    pub project_key: Option<String>,
    pub project_name: Option<String>,
    pub display_name: Option<String>,
    pub name: Option<String>,
}

// ----------------------------------------------------------------
// List Board Issues
// ----------------------------------------------------------------

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct ListBoardIssuesRequest {
    pub board_id: u64,
    pub jql: Option<String>,
    pub max_results: Option<u32>,
    pub start_at: Option<u32>,
    pub fields: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
#[serde(rename_all = "camelCase", bound(deserialize = "F: Deserialize<'de>"))]
pub struct ListBoardIssuesResponse<F = Value> {
    #[allow(dead_code)]
    #[serde(default)]
    pub start_at: u32,
    #[allow(dead_code)]
    #[serde(default)]
    pub max_results: u32,
    #[allow(dead_code)]
    pub total: Option<u32>,
    #[allow(dead_code)]
    pub is_last: Option<bool>,
    #[serde(default)]
    pub issues: Vec<IssueResponse<F>>,
}

// ----------------------------------------------------------------
// Board Configuration
// ----------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct GetBoardConfigurationRequest {
    pub board_id: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetBoardConfigurationResponse {
    pub filter: BoardFilterResponse,
    pub sub_query: BoardSubQueryResponse,
}

#[derive(Debug, Deserialize)]
pub struct BoardFilterResponse {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct BoardSubQueryResponse {
    pub query: String,
}
