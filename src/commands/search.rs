use crate::cli::SearchArgs;
use crate::client::{
    JiraClient,
    types::{GetBoardConfigurationRequest, SearchIssuesRequest, SearchIssuesResponse},
};
use crate::jql::{BoardJqlFilter, IssueQuery};
use crate::{error::AppError, render};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};

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
struct SearchIssueFields {
    summary: String,
    status: IssueStatus,
    #[serde(default)]
    components: Vec<IssueComponent>,
}

#[derive(Debug)]
struct SearchCommand {
    jql: String,
    max_results: u32,
    next_page_token: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct SearchOutput {
    issues: Vec<SearchIssueOutput>,
    is_last: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_page_token: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct SearchIssueOutput {
    key: String,
    summary: String,
    status_name: String,
    components: Vec<String>,
}

pub fn run(client: &JiraClient, args: &SearchArgs) -> Result<(), AppError> {
    let command = SearchCommand::from_args(client, args)?;
    let output = execute(client, &command)?;

    if args.json {
        render::render_json(io::stdout().lock(), &output)?;
    } else {
        render_human(io::stdout().lock(), &output)?;
    }

    Ok(())
}

impl SearchCommand {
    fn from_args(client: &JiraClient, args: &SearchArgs) -> Result<Self, AppError> {
        let configured_board_id = args.board.or_else(|| client.default_board_id());
        if configured_board_id.is_none() && !has_explicit_search_restriction(args) {
            return Err(AppError::InvalidSearch {
                reason: "provide at least one search restriction, such as --jql, --board, --project, --assignee, --component, --status, --label, --text, or configure default_board_id",
            });
        }

        let board_filter = configured_board_id
            .map(|board_id| board_filter(client, board_id))
            .transpose()?;

        Ok(Self {
            jql: IssueQuery::from_search_args(args, board_filter)?.to_jql(),
            max_results: args.limit,
            next_page_token: args.next_page_token.clone(),
        })
    }

    fn fields(&self) -> Vec<String> {
        vec![
            "summary".to_string(),
            "status".to_string(),
            "components".to_string(),
        ]
    }

    fn to_search_request(&self) -> SearchIssuesRequest {
        SearchIssuesRequest {
            jql: self.jql.clone(),
            next_page_token: self.next_page_token.clone(),
            max_results: Some(self.max_results),
            fields: self.fields(),
            ..Default::default()
        }
    }
}

fn has_explicit_search_restriction(args: &SearchArgs) -> bool {
    args.jql
        .as_deref()
        .is_some_and(|jql| !jql.trim().is_empty())
        || args.project.is_some()
        || args.assignee.is_some()
        || args.unassigned
        || args.reporter.is_some()
        || !args.status.is_empty()
        || args.status_category.is_some()
        || !args.issue_type.is_empty()
        || !args.component.is_empty()
        || !args.label.is_empty()
        || args.text.is_some()
        || args.open
}

fn board_filter(client: &JiraClient, board_id: u64) -> Result<BoardJqlFilter, AppError> {
    let configuration = client
        .get_board_configuration(&GetBoardConfigurationRequest { board_id })
        .map_err(|source| AppError::PrepareBoardSearch { board_id, source })?;

    Ok(BoardJqlFilter {
        filter_id: parse_board_filter_id(board_id, &configuration.filter.id)?,
        sub_query: Some(configuration.sub_query.query),
    })
}

fn parse_board_filter_id(board_id: u64, filter_id: &str) -> Result<u64, AppError> {
    filter_id
        .parse()
        .map_err(|source| AppError::InvalidBoardFilterId {
            board_id,
            filter_id: filter_id.to_string(),
            source,
        })
}

fn execute(client: &JiraClient, command: &SearchCommand) -> Result<SearchOutput, AppError> {
    let response = client
        .search_issues::<SearchIssueFields>(&command.to_search_request())
        .map_err(|source| AppError::ExecuteSearch { source })?;
    Ok(output_from_search_response(response))
}

fn output_from_search_response(response: SearchIssuesResponse<SearchIssueFields>) -> SearchOutput {
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
    fn from_issue(issue: crate::client::types::IssueResponse<SearchIssueFields>) -> Self {
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
        }
    }
}

fn render_human(mut writer: impl Write, output: &SearchOutput) -> Result<(), AppError> {
    if output.issues.is_empty() {
        writeln!(writer, "No issues found.").map_err(|source| AppError::RenderOutput { source })?;
    } else {
        for issue in &output.issues {
            let components = issue.components.join(", ");

            writeln!(
                writer,
                "{} [{}] {}{}",
                issue.key,
                issue.status_name,
                issue.summary,
                if components.is_empty() {
                    String::new()
                } else {
                    format!(" ({components})")
                }
            )
            .map_err(|source| AppError::RenderOutput { source })?;
        }
    }

    if !output.is_last
        && let Some(next_page_token) = &output.next_page_token
    {
        writeln!(writer, "Next page token: {next_page_token}")
            .map_err(|source| AppError::RenderOutput { source })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{JiraAuth, JiraClientConfig};
    use std::fs;
    use std::path::Path;
    use std::time::Duration;
    use url::Url;

    fn fixture(path: &str) -> String {
        fs::read_to_string(Path::new("tests/fixtures/jira").join(path)).unwrap()
    }

    fn test_client(default_board_id: Option<u64>) -> JiraClient {
        JiraClient::new(JiraClientConfig {
            base_url: Url::parse("https://example.atlassian.net/").unwrap(),
            auth: JiraAuth::Bearer {
                token: "secret".to_string(),
            },
            timeout: Duration::from_secs(30),
            default_board_id,
        })
    }

    #[test]
    fn search_requires_an_explicit_or_configured_restriction() {
        let client = test_client(None);
        let error = SearchCommand::from_args(&client, &SearchArgs::default()).unwrap_err();

        assert!(matches!(error, AppError::InvalidSearch { .. }));
    }

    #[test]
    fn invalid_board_filter_id_is_reported_instead_of_falling_back_to_board_id() {
        let error = parse_board_filter_id(215, "not-a-filter-id").unwrap_err();

        assert!(matches!(
            error,
            AppError::InvalidBoardFilterId {
                board_id: 215,
                filter_id,
                ..
            } if filter_id == "not-a-filter-id"
        ));
    }

    #[test]
    fn search_request_contains_expected_fields() {
        let client = test_client(None);
        let args = SearchArgs {
            assignee: Some("me".to_string()),
            ..Default::default()
        };
        let command = SearchCommand::from_args(&client, &args).unwrap();
        let request = command.to_search_request();

        assert_eq!(
            request.jql,
            "assignee = currentUser() ORDER BY updated DESC"
        );
        assert_eq!(request.max_results, Some(50));
        assert_eq!(request.fields, vec!["summary", "status", "components"]);
    }

    #[test]
    fn search_request_uses_pagination_args() {
        let client = test_client(None);
        let args = SearchArgs {
            assignee: Some("me".to_string()),
            limit: 25,
            next_page_token: Some("token-123".to_string()),
            ..Default::default()
        };
        let command = SearchCommand::from_args(&client, &args).unwrap();
        let request = command.to_search_request();

        assert_eq!(request.max_results, Some(25));
        assert_eq!(request.next_page_token, Some("token-123".to_string()));
    }

    #[test]
    fn deserializes_realistic_search_fixture_into_output() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-basic.json")).unwrap();

        let output = output_from_search_response(response);

        assert_eq!(output.issues.len(), 3);
        assert_eq!(output.issues[0].key, "DEMO-101");
        assert_eq!(output.issues[0].status_name, "In Review");
        assert_eq!(output.issues[0].components, vec!["Web Platform"]);
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
    fn render_human_includes_components_when_present() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-basic.json")).unwrap();
        let output = output_from_search_response(response);
        let mut rendered = Vec::new();

        render_human(&mut rendered, &output).unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            concat!(
                "DEMO-101 [In Review] Align application CSP with CDN configuration (Web Platform)\n",
                "DEMO-102 [Closed] Support iframe parent messaging (Web Platform)\n",
                "DEMO-103 [Closed] Adjust embedded content height (Web Platform)\n",
                "Next page token: sanitized-next-page-token\n"
            )
        );
    }

    #[test]
    fn render_human_omits_empty_components_suffix() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-no-components.json")).unwrap();
        let output = output_from_search_response(response);
        let mut rendered = Vec::new();

        render_human(&mut rendered, &output).unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            "DEMO-104 [Closed] Populate missing environment values\n"
        );
    }

    #[test]
    fn render_human_shows_empty_state_when_no_issues_match() {
        let output = SearchOutput {
            issues: Vec::new(),
            is_last: true,
            next_page_token: None,
        };
        let mut rendered = Vec::new();

        render_human(&mut rendered, &output).unwrap();

        assert_eq!(String::from_utf8(rendered).unwrap(), "No issues found.\n");
    }

    #[test]
    fn render_human_shows_next_page_token_when_available() {
        let output = SearchOutput {
            issues: Vec::new(),
            is_last: false,
            next_page_token: Some("abc".to_string()),
        };
        let mut rendered = Vec::new();

        render_human(&mut rendered, &output).unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            "No issues found.\nNext page token: abc\n"
        );
    }

    #[test]
    fn render_json_emits_stable_jeera_owned_schema() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-basic.json")).unwrap();
        let output = output_from_search_response(response);
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
