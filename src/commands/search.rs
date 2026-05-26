use crate::cli::SearchArgs;
use crate::client::{JiraClient, types::SearchIssuesRequest};
use crate::error::AppError;
use serde::Deserialize;

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
    render_human(&output);
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

    Ok(SearchOutput { issues })
}

fn render_human(output: &SearchOutput) {
    for issue in &output.issues {
        let components = issue.components.join(", ");

        println!(
            "{} [{}] {}{}",
            issue.key,
            issue.status_name,
            issue.summary,
            if components.is_empty() {
                String::new()
            } else {
                format!(" ({components})")
            }
        );
    }
}
