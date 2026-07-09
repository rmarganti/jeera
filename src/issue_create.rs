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
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};

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
    validate_optional("project", args.project.as_deref())
        .map_err(|reason| AppError::InvalidCreate { reason })?;
    validate_optional("board", args.board.as_deref())
        .map_err(|reason| AppError::InvalidCreate { reason })?;
    validate_required("type", &args.issue_type)
        .map_err(|reason| AppError::InvalidCreate { reason })?;
    validate_required("summary", &args.summary)
        .map_err(|reason| AppError::InvalidCreate { reason })?;
    validate_optional("body", args.body.as_deref())
        .map_err(|reason| AppError::InvalidCreate { reason })?;
    validate_optional("body-file", args.body_file.as_deref())
        .map_err(|reason| AppError::InvalidCreate { reason })?;
    validate_repeated("component", &args.component)
        .map_err(|reason| AppError::InvalidCreate { reason })?;
    validate_repeated("label", &args.label).map_err(|reason| AppError::InvalidCreate { reason })?;

    let project_key = resolve_project_key(client, args)?;
    let body = read_body(args)?;

    let mut fields = BTreeMap::from([
        ("project".to_string(), json!({ "key": project_key })),
        (
            "issuetype".to_string(),
            json!({ "name": args.issue_type.trim() }),
        ),
        ("summary".to_string(), json!(args.summary.trim())),
    ]);

    if let Some(body) = body.as_deref().filter(|body| !body.trim().is_empty()) {
        fields.insert("description".to_string(), text_to_adf(body));
    }

    if !args.component.is_empty() {
        fields.insert(
            "components".to_string(),
            Value::Array(
                args.component
                    .iter()
                    .map(|component| json!({ "name": component.trim() }))
                    .collect(),
            ),
        );
    }

    if !args.label.is_empty() {
        fields.insert(
            "labels".to_string(),
            Value::Array(
                args.label
                    .iter()
                    .map(|label| Value::String(label.trim().to_string()))
                    .collect(),
            ),
        );
    }

    for field in &args.field {
        let (key, value) =
            parse_field_assignment(field).map_err(|reason| AppError::InvalidCreate { reason })?;
        if fields.contains_key(&key) {
            return Err(AppError::InvalidCreate {
                reason: format!("--field cannot override built-in field {key:?}"),
            });
        }
        fields.insert(key, Value::String(value));
    }

    Ok(PreparedCreateIssue {
        request: CreateIssueRequest { fields },
        dry_run: args.dry_run,
    })
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

fn read_body(args: &CreateArgs) -> Result<Option<String>, AppError> {
    if let Some(body) = &args.body {
        return Ok(Some(body.clone()));
    }

    let Some(path) = &args.body_file else {
        return Ok(None);
    };

    if path == "-" {
        let mut body = String::new();
        io::stdin()
            .read_to_string(&mut body)
            .map_err(|source| AppError::ReadInput { source })?;
        return Ok(Some(body));
    }

    fs::read_to_string(path)
        .map(Some)
        .map_err(|source| AppError::ReadInput { source })
}

fn parse_field_assignment(value: &str) -> Result<(String, String), String> {
    let Some((key, value)) = value.split_once('=') else {
        return Err("--field values must use KEY=VALUE".to_string());
    };
    let key = key.trim();
    if key.is_empty() {
        return Err("--field keys cannot be empty".to_string());
    }
    if value.trim().is_empty() {
        return Err(format!("--field {key} value cannot be empty"));
    }
    Ok((key.to_string(), value.trim().to_string()))
}

fn text_to_adf(text: &str) -> Value {
    let content = text
        .split("\n\n")
        .filter_map(|paragraph| {
            let paragraph = paragraph.trim();
            if paragraph.is_empty() {
                return None;
            }

            let mut paragraph_content = Vec::new();
            for (index, line) in paragraph.lines().enumerate() {
                if index > 0 {
                    paragraph_content.push(json!({ "type": "hardBreak" }));
                }
                if !line.is_empty() {
                    paragraph_content.push(json!({ "type": "text", "text": line }));
                }
            }

            Some(json!({ "type": "paragraph", "content": paragraph_content }))
        })
        .collect::<Vec<_>>();

    let mut document = Map::new();
    document.insert("type".to_string(), Value::String("doc".to_string()));
    document.insert("version".to_string(), json!(1));
    document.insert("content".to_string(), Value::Array(content));
    Value::Object(document)
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

fn validate_required(name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("--{name} cannot be empty"))
    } else {
        Ok(())
    }
}

fn validate_optional(name: &str, value: Option<&str>) -> Result<(), String> {
    match value {
        Some(value) => validate_required(name, value),
        None => Ok(()),
    }
}

fn validate_repeated(name: &str, values: &[String]) -> Result<(), String> {
    if values.iter().any(|value| value.trim().is_empty()) {
        Err(format!("--{name} cannot contain empty values"))
    } else {
        Ok(())
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

    #[test]
    fn adf_preserves_line_breaks_inside_paragraphs() {
        let adf = text_to_adf("One\nTwo\n\nThree");

        assert_eq!(adf["type"], "doc");
        assert_eq!(adf["content"].as_array().unwrap().len(), 2);
        assert_eq!(adf["content"][0]["content"][1]["type"], "hardBreak");
    }

    #[test]
    fn field_assignment_requires_key_value_shape() {
        assert!(parse_field_assignment("missing").is_err());
        assert!(parse_field_assignment("=value").is_err());
        assert!(parse_field_assignment("key= ").is_err());
        assert_eq!(
            parse_field_assignment("customfield_1=value").unwrap(),
            ("customfield_1".to_string(), "value".to_string())
        );
    }
}
