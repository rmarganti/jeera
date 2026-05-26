use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

// ----------------------------------------------------------------
// Common
// ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "F: Deserialize<'de>"))]
pub struct IssueResponse<F = Value> {
    pub id: String,
    pub key: String,
    #[serde(rename = "self")]
    pub self_link: String,
    pub fields: F,
}

#[derive(Debug)]
pub enum JiraError {
    Http(String),
    HttpStatus { status: u16, body: String },
    Json(String),
}

impl fmt::Display for JiraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JiraError::Http(message) => write!(f, "http error: {message}"),
            JiraError::HttpStatus { status, body } => {
                write!(f, "jira returned HTTP {status}: {body}")
            }
            JiraError::Json(message) => write!(f, "json error: {message}"),
        }
    }
}

impl std::error::Error for JiraError {}

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
    #[serde(default)]
    pub names: BTreeMap<String, String>,
    pub next_page_token: Option<String>,
    #[serde(default)]
    pub schema: BTreeMap<String, Value>,
}
