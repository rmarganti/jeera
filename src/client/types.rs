use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

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

#[derive(Debug, Error)]
pub enum JiraError {
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
    #[error("jira returned HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },
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
