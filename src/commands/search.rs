use crate::cli::SearchArgs;
use crate::client::{
    JiraClient,
    types::{SearchIssuesRequest, SearchIssuesResponse},
};
use crate::error::AppError;
use serde::Deserialize;
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
}

#[derive(Debug)]
struct SearchOutput {
    issues: Vec<SearchIssueOutput>,
}

#[derive(Debug)]
struct SearchIssueOutput {
    key: String,
    summary: String,
    status_name: String,
    components: Vec<String>,
}

pub fn run(client: &JiraClient, args: &SearchArgs) -> Result<(), AppError> {
    let command = SearchCommand::from_args(args);
    let output = execute(client, &command)?;
    render_human(io::stdout().lock(), &output)?;
    Ok(())
}

impl SearchCommand {
    fn from_args(_args: &SearchArgs) -> Self {
        Self {
            jql: "assignee = currentUser() ORDER BY updated DESC".to_string(),
            max_results: 5,
        }
    }

    fn to_request(&self) -> SearchIssuesRequest {
        SearchIssuesRequest {
            jql: self.jql.clone(),
            max_results: Some(self.max_results),
            fields: vec![
                "summary".to_string(),
                "status".to_string(),
                "components".to_string(),
            ],
            ..Default::default()
        }
    }
}

fn execute(client: &JiraClient, command: &SearchCommand) -> Result<SearchOutput, AppError> {
    let response = client
        .search_issues::<SearchIssueFields>(&command.to_request())
        .map_err(|source| AppError::ExecuteSearch { source })?;

    Ok(output_from_response(response))
}

fn output_from_response(response: SearchIssuesResponse<SearchIssueFields>) -> SearchOutput {
    let issues = response
        .issues
        .into_iter()
        .map(|issue| SearchIssueOutput {
            key: issue.key,
            summary: issue.fields.summary,
            status_name: issue.fields.status.name,
            components: issue
                .fields
                .components
                .into_iter()
                .map(|component| component.name)
                .collect(),
        })
        .collect();

    SearchOutput { issues }
}

fn render_human(mut writer: impl Write, output: &SearchOutput) -> Result<(), AppError> {
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

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn fixture(path: &str) -> String {
        fs::read_to_string(Path::new("tests/fixtures/jira").join(path)).unwrap()
    }

    #[test]
    fn search_request_contains_expected_fields() {
        let command = SearchCommand::from_args(&SearchArgs {});
        let request = command.to_request();

        assert_eq!(
            request.jql,
            "assignee = currentUser() ORDER BY updated DESC"
        );
        assert_eq!(request.max_results, Some(5));
        assert_eq!(request.fields, vec!["summary", "status", "components"]);
    }

    #[test]
    fn deserializes_realistic_search_fixture_into_output() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-basic.json")).unwrap();

        let output = output_from_response(response);

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
        let output = output_from_response(response);
        let mut rendered = Vec::new();

        render_human(&mut rendered, &output).unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            concat!(
                "DEMO-101 [In Review] Align application CSP with CDN configuration (Web Platform)\n",
                "DEMO-102 [Closed] Support iframe parent messaging (Web Platform)\n",
                "DEMO-103 [Closed] Adjust embedded content height (Web Platform)\n"
            )
        );
    }

    #[test]
    fn render_human_omits_empty_components_suffix() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-no-components.json")).unwrap();
        let output = output_from_response(response);
        let mut rendered = Vec::new();

        render_human(&mut rendered, &output).unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            "DEMO-104 [Closed] Populate missing environment values\n"
        );
    }
}
