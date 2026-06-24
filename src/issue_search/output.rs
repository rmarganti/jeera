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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue_search::tests_support::{fixture, parse_fixture};
    use crate::render;

    #[test]
    fn deserializes_realistic_search_fixture_into_output() {
        let output = parse_fixture("search-basic.json");

        assert_eq!(output.issues.len(), 3);
        assert_eq!(output.issues[0].key, "DEMO-101");
        assert_eq!(output.issues[0].status_name, "In Review");
        assert_eq!(output.issues[0].components, vec!["Web Platform"]);
    }

    #[test]
    fn deserializes_selected_optional_columns_when_present() {
        let output = parse_fixture("search-columns.json");

        assert_eq!(output.issues[0].issue_type_name.as_deref(), Some("Bug"));
        assert_eq!(
            output.issues[0].assignee_display_name.as_deref(),
            Some("Mina Li")
        );
        assert_eq!(output.issues[0].priority_name.as_deref(), Some("High"));
        assert_eq!(output.issues[1].assignee_display_name.as_deref(), None);
        assert_eq!(output.issues[1].priority_name.as_deref(), None);
    }

    #[test]
    fn deserialization_fails_when_required_summary_is_missing() {
        let error = serde_json::from_str::<SearchIssuesResponse<SearchIssueFields>>(&fixture(
            "search-missing-summary.json",
        ))
        .unwrap_err();

        assert!(error.to_string().contains("summary"));
    }

    #[test]
    fn deserialization_fails_when_status_shape_changes() {
        let error = serde_json::from_str::<SearchIssuesResponse<SearchIssueFields>>(&fixture(
            "search-invalid-status-shape.json",
        ))
        .unwrap_err();

        assert_eq!(error.classify(), serde_json::error::Category::Data);
    }

    #[test]
    fn render_json_emits_stable_jeera_owned_schema() {
        let output = parse_fixture("search-basic.json");
        let mut rendered = Vec::new();

        render::render_json(&mut rendered, &output).unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            concat!(
                "{\n",
                "  \"issues\": [\n",
                "    {\n",
                "      \"key\": \"DEMO-101\",\n",
                "      \"summary\": \"Align application CSP with CDN configuration\",\n",
                "      \"status_name\": \"In Review\",\n",
                "      \"components\": [\n",
                "        \"Web Platform\"\n",
                "      ]\n",
                "    },\n",
                "    {\n",
                "      \"key\": \"DEMO-102\",\n",
                "      \"summary\": \"Support iframe parent messaging\",\n",
                "      \"status_name\": \"Closed\",\n",
                "      \"components\": [\n",
                "        \"Web Platform\"\n",
                "      ]\n",
                "    },\n",
                "    {\n",
                "      \"key\": \"DEMO-103\",\n",
                "      \"summary\": \"Adjust embedded content height\",\n",
                "      \"status_name\": \"Closed\",\n",
                "      \"components\": [\n",
                "        \"Web Platform\"\n",
                "      ]\n",
                "    }\n",
                "  ],\n",
                "  \"is_last\": false,\n",
                "  \"next_page_token\": \"sanitized-next-page-token\"\n",
                "}\n"
            )
        );
    }

    #[test]
    fn render_json_emits_additive_optional_fields_when_available() {
        let output = parse_fixture("search-columns.json");
        let mut rendered = Vec::new();

        render::render_json(&mut rendered, &output).unwrap();

        let rendered = String::from_utf8(rendered).unwrap();
        assert!(rendered.contains("\"issue_type_name\": \"Bug\""));
        assert!(rendered.contains("\"assignee_display_name\": \"Mina Li\""));
        assert!(rendered.contains("\"priority_name\": \"High\""));
        assert!(rendered.contains("\"updated\": \"2026-06-22T14:45:00.000+0000\""));
    }

    #[test]
    fn render_json_emits_empty_collection_for_no_matches() {
        let output = SearchOutput {
            issues: Vec::new(),
            is_last: true,
            next_page_token: None,
        };
        let mut rendered = Vec::new();

        render::render_json(&mut rendered, &output).unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            "{\n  \"issues\": [],\n  \"is_last\": true\n}\n"
        );
    }
}
