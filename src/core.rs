use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// A summary of a Jira issue. Primarily used when listing/searching Issues.
/// For the full details of an Issue, see `Issue`.
#[derive(Debug, Deserialize)]
pub struct IssueSummary {
    pub id: String,
    pub key: String,
    #[serde(rename = "self")]
    pub self_link: String,
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
}
