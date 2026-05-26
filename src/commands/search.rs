use crate::cli::SearchArgs;
use crate::client::types::SearchIssuesResponse;
use crate::client::{JiraClient, types::SearchIssuesRequest};
use crate::error::AppError;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueStatusCategory {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueStatus {
    pub name: String,
    pub status_category: IssueStatusCategory,
}

#[derive(Debug, Deserialize)]
pub struct IssueComponent {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchIssueFields {
    pub summary: String,
    pub status: IssueStatus,
    #[serde(default)]
    pub components: Vec<IssueComponent>,
}

pub fn run(client: &JiraClient, _args: &SearchArgs) -> Result<(), AppError> {
    let request = SearchIssuesRequest {
        jql: "assignee = currentUser() ORDER BY updated DESC".to_string(),
        max_results: Some(5),
        fields: vec![
            "summary".to_string(),
            "status".to_string(),
            "components".to_string(),
        ],
        ..Default::default()
    };

    let response = client
        .search_issues::<SearchIssueFields>(&request)
        .map_err(|source| AppError::ExecuteSearch { source })?;

    print_results(response);
    Ok(())
}

fn print_results(response: SearchIssuesResponse<SearchIssueFields>) {
    for issue in response.issues {
        let components = issue
            .fields
            .components
            .iter()
            .map(|component| component.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        println!(
            "{} [{}] {}{}",
            issue.key,
            issue.fields.status.name,
            issue.fields.summary,
            if components.is_empty() {
                String::new()
            } else {
                format!(" ({components})")
            }
        );
    }
}
