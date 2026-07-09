//! Jira metadata/read-only workflow capability module.
//!
//! These commands expose Jira's project/issue metadata needed before write commands
//! can safely create, edit, or transition issues.

use crate::board::{self, BoardSelector};
use crate::cli::{ShowCreateMetaArgs, ShowEditMetaArgs, ShowTransitionsArgs};
use crate::client::{
    JiraClient,
    types::{
        FieldMetadataResponse, GetCreateMetaRequest, GetCreateMetaResponse, GetEditMetaRequest,
        GetEditMetaResponse, GetTransitionsRequest, GetTransitionsResponse, ListBoardsRequest,
    },
};
use crate::error::AppError;
use serde::Serialize;
use std::io::Write;

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateMetaOutput {
    projects: Vec<CreateMetaProjectOutput>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct CreateMetaProjectOutput {
    id: Option<String>,
    key: String,
    name: String,
    issue_types: Vec<CreateMetaIssueTypeOutput>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct CreateMetaIssueTypeOutput {
    id: String,
    name: String,
    description: Option<String>,
    subtask: bool,
    required_fields: Vec<FieldOutput>,
    optional_fields: Vec<FieldOutput>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct EditMetaOutput {
    issue_key: String,
    fields: Vec<FieldOutput>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct FieldOutput {
    key: String,
    name: String,
    required: bool,
    operations: Vec<String>,
    schema_type: Option<String>,
    allowed_values_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct TransitionsOutput {
    issue_key: String,
    transitions: Vec<TransitionOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedCreateMeta {
    request: GetCreateMetaRequest,
    #[allow(dead_code)]
    project_source: ProjectSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProjectSource {
    ExplicitProject,
    ExplicitBoard { board_id: u64 },
    DefaultBoard { board_id: u64 },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct TransitionOutput {
    id: String,
    name: String,
    to_status: String,
    to_status_category: Option<String>,
    has_screen: bool,
    is_available: Option<bool>,
}

pub fn show_create_meta(
    client: &JiraClient,
    args: &ShowCreateMetaArgs,
) -> Result<CreateMetaOutput, AppError> {
    let prepared = prepare_show_create_meta(client, args)?;

    let response = client
        .get_create_meta(&prepared.request)
        .map_err(|source| AppError::ExecuteMetadata { source })?;

    Ok(create_meta_output(response))
}

pub fn show_edit_meta(
    client: &JiraClient,
    args: &ShowEditMetaArgs,
) -> Result<EditMetaOutput, AppError> {
    validate_required("issue-key", &args.issue_key)
        .map_err(|reason| AppError::InvalidMetadata { reason })?;

    let response = client
        .get_edit_meta(&GetEditMetaRequest {
            issue_id_or_key: args.issue_key.clone(),
        })
        .map_err(|source| AppError::ExecuteMetadata { source })?;

    Ok(edit_meta_output(args.issue_key.clone(), response))
}

pub fn show_transitions(
    client: &JiraClient,
    args: &ShowTransitionsArgs,
) -> Result<TransitionsOutput, AppError> {
    validate_required("issue-key", &args.issue_key)
        .map_err(|reason| AppError::InvalidMetadata { reason })?;

    let response = client
        .get_transitions(&GetTransitionsRequest {
            issue_id_or_key: args.issue_key.clone(),
        })
        .map_err(|source| AppError::ExecuteMetadata { source })?;

    Ok(transitions_output(args.issue_key.clone(), response))
}

pub fn render_create_meta_human(
    mut writer: impl Write,
    output: &CreateMetaOutput,
) -> Result<(), AppError> {
    if output.projects.is_empty() {
        writeln!(writer, "No create metadata found.")
            .map_err(|source| AppError::RenderOutput { source })?;
        return Ok(());
    }

    for project in &output.projects {
        writeln!(writer, "Project: {} ({})", project.key, project.name)
            .map_err(|source| AppError::RenderOutput { source })?;
        writeln!(writer).map_err(|source| AppError::RenderOutput { source })?;
        writeln!(writer, "Issue types:").map_err(|source| AppError::RenderOutput { source })?;

        if project.issue_types.is_empty() {
            writeln!(writer, "No issue types found.")
                .map_err(|source| AppError::RenderOutput { source })?;
            continue;
        }

        for issue_type in &project.issue_types {
            writeln!(writer, "- {} [{}]", issue_type.name, issue_type.id)
                .map_err(|source| AppError::RenderOutput { source })?;
            if !issue_type.required_fields.is_empty() {
                writeln!(writer, "  Required fields:")
                    .map_err(|source| AppError::RenderOutput { source })?;
                for field in &issue_type.required_fields {
                    write_field(&mut writer, field, "    - ")?;
                }
            }
        }
    }

    Ok(())
}

pub fn render_edit_meta_human(
    mut writer: impl Write,
    output: &EditMetaOutput,
) -> Result<(), AppError> {
    writeln!(writer, "Editable fields for {}:", output.issue_key)
        .map_err(|source| AppError::RenderOutput { source })?;

    if output.fields.is_empty() {
        writeln!(writer, "No editable fields found.")
            .map_err(|source| AppError::RenderOutput { source })?;
        return Ok(());
    }

    for field in &output.fields {
        write_field(&mut writer, field, "- ")?;
    }

    Ok(())
}

pub fn render_transitions_human(
    mut writer: impl Write,
    output: &TransitionsOutput,
) -> Result<(), AppError> {
    writeln!(writer, "Transitions for {}:", output.issue_key)
        .map_err(|source| AppError::RenderOutput { source })?;

    if output.transitions.is_empty() {
        writeln!(writer, "No transitions available.")
            .map_err(|source| AppError::RenderOutput { source })?;
        return Ok(());
    }

    for transition in &output.transitions {
        let category = transition
            .to_status_category
            .as_deref()
            .map(|category| format!(" ({category})"))
            .unwrap_or_default();
        writeln!(
            writer,
            "- {} [{}] -> {}{}",
            transition.name, transition.id, transition.to_status, category
        )
        .map_err(|source| AppError::RenderOutput { source })?;
    }

    Ok(())
}

fn prepare_show_create_meta(
    client: &JiraClient,
    args: &ShowCreateMetaArgs,
) -> Result<PreparedCreateMeta, AppError> {
    validate_optional("project", args.project.as_deref())
        .map_err(|reason| AppError::InvalidMetadata { reason })?;
    validate_optional("board", args.board.as_deref())
        .map_err(|reason| AppError::InvalidMetadata { reason })?;
    validate_optional("type", args.issue_type.as_deref())
        .map_err(|reason| AppError::InvalidMetadata { reason })?;

    let (project_key, project_source) = resolve_create_meta_project(client, args)?;

    Ok(PreparedCreateMeta {
        request: GetCreateMetaRequest {
            project_key,
            issue_type_name: args.issue_type.clone(),
        },
        project_source,
    })
}

fn resolve_create_meta_project(
    client: &JiraClient,
    args: &ShowCreateMetaArgs,
) -> Result<(String, ProjectSource), AppError> {
    if let Some(project) = &args.project {
        return Ok((project.clone(), ProjectSource::ExplicitProject));
    }

    let explicit_board = args
        .board
        .as_deref()
        .map(str::trim)
        .map(|board| {
            board::parse_board_selector(board, "board")
                .map_err(|reason| AppError::InvalidMetadata { reason })
        })
        .transpose()?;

    match explicit_board {
        Some(BoardSelector::Id(board_id)) => {
            let project_key = project_key_for_board_id(client, board_id)?;
            Ok((project_key, ProjectSource::ExplicitBoard { board_id }))
        }
        Some(BoardSelector::Name(board_name)) => {
            let boards = load_boards(client)?;
            let board_id = board::find_board_id_by_name(&boards, &board_name)
                .map_err(|reason| AppError::InvalidMetadata { reason })?;
            let project_key = board::find_board_project_key(&boards, board_id)
                .map_err(|reason| AppError::InvalidMetadata { reason })?;
            Ok((project_key, ProjectSource::ExplicitBoard { board_id }))
        }
        None => {
            let Some(board_id) = client.default_board_id() else {
                return Err(AppError::InvalidMetadata {
                    reason: "provide --project, --board, or configure default_board_id".to_string(),
                });
            };
            let project_key = project_key_for_board_id(client, board_id)?;
            Ok((project_key, ProjectSource::DefaultBoard { board_id }))
        }
    }
}

fn project_key_for_board_id(client: &JiraClient, board_id: u64) -> Result<String, AppError> {
    let boards = load_boards(client)?;
    board::find_board_project_key(&boards, board_id)
        .map_err(|reason| AppError::InvalidMetadata { reason })
}

fn load_boards(client: &JiraClient) -> Result<Vec<crate::client::types::BoardResponse>, AppError> {
    client
        .list_boards(&ListBoardsRequest::default())
        .map(|response| response.values)
        .map_err(|source| AppError::ExecuteMetadata { source })
}

fn create_meta_output(response: GetCreateMetaResponse) -> CreateMetaOutput {
    CreateMetaOutput {
        projects: response
            .projects
            .into_iter()
            .map(|project| CreateMetaProjectOutput {
                id: project.id,
                key: project.key,
                name: project.name,
                issue_types: project
                    .issuetypes
                    .into_iter()
                    .map(|issue_type| {
                        let mut fields = issue_type
                            .fields
                            .into_values()
                            .map(field_output)
                            .collect::<Vec<_>>();
                        fields.sort_by(|left, right| left.name.cmp(&right.name));
                        let (required_fields, optional_fields) =
                            fields.into_iter().partition(|field| field.required);

                        CreateMetaIssueTypeOutput {
                            id: issue_type.id,
                            name: issue_type.name,
                            description: issue_type.description,
                            subtask: issue_type.subtask,
                            required_fields,
                            optional_fields,
                        }
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn edit_meta_output(issue_key: String, response: GetEditMetaResponse) -> EditMetaOutput {
    let mut fields = response
        .fields
        .into_values()
        .map(field_output)
        .collect::<Vec<_>>();
    fields.sort_by(|left, right| left.name.cmp(&right.name));

    EditMetaOutput { issue_key, fields }
}

fn transitions_output(issue_key: String, response: GetTransitionsResponse) -> TransitionsOutput {
    TransitionsOutput {
        issue_key,
        transitions: response
            .transitions
            .into_iter()
            .map(|transition| TransitionOutput {
                id: transition.id,
                name: transition.name,
                to_status: transition.to.name,
                to_status_category: transition.to.status_category.map(|category| category.name),
                has_screen: transition.has_screen,
                is_available: transition.is_available,
            })
            .collect(),
    }
}

fn field_output(field: FieldMetadataResponse) -> FieldOutput {
    FieldOutput {
        key: field.key,
        name: field.name,
        required: field.required,
        operations: field.operations,
        schema_type: field.schema.as_ref().and_then(schema_type),
        allowed_values_count: field.allowed_values.len(),
    }
}

fn schema_type(schema: &serde_json::Value) -> Option<String> {
    schema
        .get("type")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            schema
                .get("custom")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
}

fn write_field(writer: &mut impl Write, field: &FieldOutput, prefix: &str) -> Result<(), AppError> {
    let schema = field
        .schema_type
        .as_deref()
        .map(|schema| format!("; {schema}"))
        .unwrap_or_default();
    let operations = if field.operations.is_empty() {
        String::new()
    } else {
        format!("; ops: {}", field.operations.join(","))
    };

    writeln!(
        writer,
        "{}{} ({}){}{}",
        prefix, field.name, field.key, schema, operations
    )
    .map_err(|source| AppError::RenderOutput { source })
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
    fn create_meta_prefers_explicit_project() {
        let client = test_client(Some(215));
        let prepared = prepare_show_create_meta(
            &client,
            &ShowCreateMetaArgs {
                project: Some("GCCDEV".to_string()),
                board: None,
                issue_type: Some("Task".to_string()),
                json: false,
            },
        )
        .unwrap();

        assert_eq!(prepared.request.project_key, "GCCDEV");
        assert_eq!(prepared.request.issue_type_name.as_deref(), Some("Task"));
        assert_eq!(prepared.project_source, ProjectSource::ExplicitProject);
    }

    #[test]
    fn create_meta_requires_project_board_or_default_board() {
        let client = test_client(None);
        let error = prepare_show_create_meta(&client, &ShowCreateMetaArgs::default()).unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid metadata request: provide --project, --board, or configure default_board_id"
        );
    }
}
