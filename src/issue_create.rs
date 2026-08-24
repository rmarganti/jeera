//! Jira issue creation domain module.
//!
//! Creation stays CLI-first: common Jira fields are represented as explicit
//! arguments, while long descriptions can come from a body file/stdin.

use crate::board::{self, BoardSelector};
use crate::cli::CreateArgs;
use crate::client::{
    JiraClient,
    types::{CreateIssueRequest, CreateIssueResponse, GetCreateMetaRequest, ListBoardsRequest},
};
use crate::error::AppError;
use crate::issue_fields::{
    CollectionChange, CommonIssueFieldInput, PrepareIssueFieldsError, prepare_common_fields,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::io::Write;

#[derive(Debug)]
pub struct PreparedCreateIssue {
    request: CreateIssueRequest,
    dry_run: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateIssueOutput {
    dry_run: bool,
    project_key: String,
    issue_type: String,
    summary: String,
    key: Option<String>,
    id: Option<String>,
    url: Option<String>,
    request: CreateIssueRequest,
}

pub fn execute(client: &JiraClient, args: &CreateArgs) -> Result<CreateIssueOutput, AppError> {
    let prepared = prepare(client, args)?;
    validate_against_create_meta(client, &prepared)?;

    if prepared.dry_run {
        return Ok(output_from_prepared(prepared, None, None));
    }

    let response = client
        .create_issue(prepared.request())
        .map_err(|source| AppError::ExecuteCreate { source })?;
    let url = client
        .issue_browse_url(&response.key)
        .map_err(|source| AppError::ExecuteCreate { source })?;

    Ok(output_from_prepared(prepared, Some(response), Some(url)))
}

pub fn render_human(mut writer: impl Write, output: &CreateIssueOutput) -> Result<(), AppError> {
    if output.dry_run {
        writeln!(writer, "Dry run: issue was not created.")
            .map_err(|source| AppError::RenderOutput { source })?;
        writeln!(
            writer,
            "Would create {} {}: {}",
            output.project_key, output.issue_type, output.summary
        )
        .map_err(|source| AppError::RenderOutput { source })?;
    } else {
        writeln!(
            writer,
            "Created {}: {}",
            output.key.as_deref().unwrap_or("<unknown>"),
            output.summary
        )
        .map_err(|source| AppError::RenderOutput { source })?;
        if let Some(url) = &output.url {
            writeln!(writer, "URL: {url}").map_err(|source| AppError::RenderOutput { source })?;
        }
    }

    Ok(())
}

impl PreparedCreateIssue {
    pub fn request(&self) -> &CreateIssueRequest {
        &self.request
    }
}

fn prepare(client: &JiraClient, args: &CreateArgs) -> Result<PreparedCreateIssue, AppError> {
    validate_create_arg("project", args.project.as_deref())?;
    validate_create_arg("board", args.board.as_deref())?;
    validate_create_arg("type", Some(&args.issue_type))?;
    validate_create_arg("summary", Some(&args.summary))?;

    let project_key = resolve_project_key(client, args)?;
    let mut fields = prepare_common_fields(CommonIssueFieldInput {
        summary: Some(&args.summary),
        body: args.body.as_deref(),
        body_file: args.body_file.as_deref(),
        clear_body: false,
        components: if args.component.is_empty() {
            CollectionChange::Unchanged
        } else {
            CollectionChange::Set(&args.component)
        },
        labels: if args.label.is_empty() {
            CollectionChange::Unchanged
        } else {
            CollectionChange::Set(&args.label)
        },
        custom_fields: &args.field,
        reserved_fields: &[
            "project",
            "issuetype",
            "summary",
            "description",
            "components",
            "labels",
        ],
    })
    .map_err(map_field_error)?;
    fields.insert("project".to_string(), json!({ "key": project_key }));
    fields.insert(
        "issuetype".to_string(),
        json!({ "name": args.issue_type.trim() }),
    );

    Ok(PreparedCreateIssue {
        request: CreateIssueRequest { fields },
        dry_run: args.dry_run,
    })
}

fn validate_create_arg(name: &str, value: Option<&str>) -> Result<(), AppError> {
    if value.is_some_and(|value| value.trim().is_empty()) {
        Err(AppError::InvalidCreate {
            reason: format!("--{name} cannot be empty"),
        })
    } else {
        Ok(())
    }
}

fn map_field_error(error: PrepareIssueFieldsError) -> AppError {
    match error {
        PrepareIssueFieldsError::Invalid(reason) => AppError::InvalidCreate { reason },
        PrepareIssueFieldsError::ReadInput { source } => AppError::ReadInput { source },
    }
}

fn validate_against_create_meta(
    client: &JiraClient,
    prepared: &PreparedCreateIssue,
) -> Result<(), AppError> {
    let project_key = prepared
        .request
        .fields
        .get("project")
        .and_then(|project| project.get("key"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let issue_type = prepared
        .request
        .fields
        .get("issuetype")
        .and_then(|issue_type| issue_type.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    let meta = client
        .get_create_meta(&GetCreateMetaRequest {
            project_key: project_key.to_string(),
            issue_type_name: Some(issue_type.to_string()),
        })
        .map_err(|source| AppError::ExecuteCreate { source })?;

    let Some(project) = meta
        .projects
        .into_iter()
        .find(|project| project.key == project_key)
    else {
        return Err(AppError::InvalidCreate {
            reason: format!("project {project_key:?} was not found in Jira create metadata"),
        });
    };

    let Some(issue_type_meta) = project
        .issuetypes
        .into_iter()
        .find(|candidate| candidate.name == issue_type)
    else {
        return Err(AppError::InvalidCreate {
            reason: format!("issue type {issue_type:?} is not available for project {project_key}"),
        });
    };

    let missing_required = issue_type_meta
        .fields
        .iter()
        .filter(|(key, field)| field.required && !prepared.request.fields.contains_key(*key))
        .map(|(key, field)| format!("{} ({key})", field.name))
        .collect::<Vec<_>>();

    if !missing_required.is_empty() {
        return Err(AppError::InvalidCreate {
            reason: format!(
                "missing required create fields: {}; provide supported fields with dedicated flags or --field KEY=VALUE",
                missing_required.join(", ")
            ),
        });
    }

    Ok(())
}

fn resolve_project_key(client: &JiraClient, args: &CreateArgs) -> Result<String, AppError> {
    if let Some(project) = &args.project {
        return Ok(project.trim().to_string());
    }

    let explicit_board = args
        .board
        .as_deref()
        .map(str::trim)
        .map(|board| {
            board::parse_board_selector(board, "board")
                .map_err(|reason| AppError::InvalidCreate { reason })
        })
        .transpose()?;

    let board_id = match explicit_board {
        Some(BoardSelector::Id(board_id)) => board_id,
        Some(BoardSelector::Name(board_name)) => {
            let boards = load_boards(client)?;
            return board::find_board_id_by_name(&boards, &board_name)
                .and_then(|board_id| board::find_board_project_key(&boards, board_id))
                .map_err(|reason| AppError::InvalidCreate { reason });
        }
        None => client
            .default_board_id()
            .ok_or_else(|| AppError::InvalidCreate {
                reason: "provide --project, --board, or configure default_board_id".to_string(),
            })?,
    };

    let boards = load_boards(client)?;
    board::find_board_project_key(&boards, board_id)
        .map_err(|reason| AppError::InvalidCreate { reason })
}

fn load_boards(client: &JiraClient) -> Result<Vec<crate::client::types::BoardResponse>, AppError> {
    client
        .list_boards(&ListBoardsRequest::default())
        .map(|response| response.values)
        .map_err(|source| AppError::ExecuteCreate { source })
}

fn output_from_prepared(
    prepared: PreparedCreateIssue,
    response: Option<CreateIssueResponse>,
    url: Option<String>,
) -> CreateIssueOutput {
    let project_key = prepared.request.fields["project"]["key"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let issue_type = prepared.request.fields["issuetype"]["name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let summary = prepared.request.fields["summary"]
        .as_str()
        .unwrap_or_default()
        .to_string();

    CreateIssueOutput {
        dry_run: prepared.dry_run,
        project_key,
        issue_type,
        summary,
        key: response.as_ref().map(|response| response.key.clone()),
        id: response.as_ref().map(|response| response.id.clone()),
        url,
        request: prepared.request,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{JiraAuth, JiraClientConfig};
    use std::collections::BTreeMap;
    use std::time::Duration;
    use url::Url;

    fn test_client(default_board_id: Option<u64>) -> JiraClient {
        JiraClient::new(JiraClientConfig {
            base_url: Url::parse("https://example.atlassian.net/").unwrap(),
            auth: JiraAuth::Bearer {
                token: "secret-token".to_string(),
            },
            timeout: Duration::from_secs(30),
            default_board_id,
            searches: BTreeMap::new(),
        })
    }

    #[test]
    fn prepares_create_request_from_cli_fields() {
        let client = test_client(None);
        let prepared = prepare(
            &client,
            &CreateArgs {
                project: Some("GCCDEV".to_string()),
                issue_type: "Task".to_string(),
                summary: "Create homes intake validation".to_string(),
                body: Some("First line\nSecond line".to_string()),
                component: vec!["Homes".to_string()],
                label: vec!["homes-2".to_string()],
                field: vec!["customfield_12345=demo".to_string()],
                dry_run: true,
                ..Default::default()
            },
        )
        .unwrap();

        let fields = &prepared.request().fields;
        assert_eq!(fields["project"]["key"], "GCCDEV");
        assert_eq!(fields["issuetype"]["name"], "Task");
        assert_eq!(fields["summary"], "Create homes intake validation");
        assert_eq!(fields["components"][0]["name"], "Homes");
        assert_eq!(fields["labels"][0], "homes-2");
        assert_eq!(fields["customfield_12345"], "demo");
        assert_eq!(fields["description"]["type"], "doc");
    }

    #[test]
    fn create_requires_project_board_or_default_board() {
        let client = test_client(None);
        let error = prepare(
            &client,
            &CreateArgs {
                issue_type: "Task".to_string(),
                summary: "Demo".to_string(),
                ..Default::default()
            },
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid create request: provide --project, --board, or configure default_board_id"
        );
    }
}
