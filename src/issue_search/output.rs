use crate::client::types::{IssueResponse, SearchIssuesResponse};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueStatus {
    name: String,
}

#[derive(Debug, Deserialize)]
struct IssueComponent {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NamedField {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserField {
    display_name: String,
}

// Jira response shape requested by `search_fields`; keep these in lockstep.
#[derive(Debug, Deserialize)]
pub(crate) struct SearchIssueFields {
    summary: String,
    status: IssueStatus,
    #[serde(default)]
    components: Vec<IssueComponent>,
    #[serde(rename = "issuetype")]
    issue_type: Option<NamedField>,
    priority: Option<NamedField>,
    assignee: Option<UserField>,
    updated: Option<String>,
}

/// jeera-owned search output; this is the stable interface for JSON and human rendering.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SearchOutput {
    pub(crate) issues: Vec<SearchIssueOutput>,
    pub(crate) is_last: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_page_token: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct SearchIssueOutput {
    pub(crate) key: String,
    pub(crate) summary: String,
    pub(crate) status_name: String,
    pub(crate) components: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) issue_type_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) assignee_display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) priority_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) updated: Option<String>,
}

impl SearchOutput {
    pub(super) fn issues(&self) -> &[SearchIssueOutput] {
        &self.issues
    }

    pub(super) fn is_last(&self) -> bool {
        self.is_last
    }

    pub(super) fn next_page_token(&self) -> Option<&str> {
        self.next_page_token.as_deref()
    }
}

pub(crate) fn output_from_search_response(
    response: SearchIssuesResponse<SearchIssueFields>,
) -> SearchOutput {
    SearchOutput {
        issues: response
            .issues
            .into_iter()
            .map(SearchIssueOutput::from_issue)
            .collect(),
        is_last: response.is_last,
        next_page_token: response.next_page_token,
    }
}

impl SearchIssueOutput {
    fn from_issue(issue: IssueResponse<SearchIssueFields>) -> Self {
        Self {
            key: issue.key,
            summary: issue.fields.summary,
            status_name: issue.fields.status.name,
            components: issue
                .fields
                .components
                .into_iter()
                .map(|component| component.name)
                .collect(),
            issue_type_name: issue.fields.issue_type.map(|issue_type| issue_type.name),
            assignee_display_name: issue.fields.assignee.map(|assignee| assignee.display_name),
            priority_name: issue.fields.priority.map(|priority| priority.name),
            updated: issue.fields.updated,
        }
    }
}
