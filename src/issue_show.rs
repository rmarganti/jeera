//! Jira show issue domain module.
//!
//! The command adapter enters through `execute`; tests and future callers can use
//! `prepare` when they only need the prepared Jira request.

use crate::cli::ShowArgs;
use crate::client::{
    JiraClient,
    types::{GetIssueRequest, GetIssueResponse},
};
use crate::error::AppError;
use crate::render;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;

/// Prepared show issue intent after field selection and option normalization.
#[derive(Debug)]
pub struct PreparedShowIssue {
    request: GetIssueRequest,
    include_comments: bool,
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

// Jira response shape requested by `show_issue_fields`; keep these in lockstep.
#[derive(Debug, Deserialize)]
struct ShowIssueFields {
    summary: String,
    status: NamedField,
    #[serde(default)]
    components: Vec<NamedField>,
    #[serde(rename = "issuetype")]
    issue_type: NamedField,
    priority: Option<NamedField>,
    assignee: Option<UserField>,
    reporter: Option<UserField>,
    created: String,
    updated: String,
    description: Option<Value>,
    #[serde(default)]
    comment: CommentPage,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CommentPage {
    #[serde(default)]
    comments: Vec<CommentField>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentField {
    id: String,
    author: Option<UserField>,
    body: Option<Value>,
    created: String,
    updated: String,
}

/// jeera-owned show issue output; this is the stable interface for JSON and human rendering.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ShowIssueOutput {
    key: String,
    summary: String,
    status_name: String,
    components: Vec<String>,
    issue_type_name: String,
    priority_name: Option<String>,
    assignee_display_name: Option<String>,
    reporter_display_name: Option<String>,
    created: String,
    updated: String,
    url: String,
    description: Option<String>,
    comments: Vec<CommentOutput>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct CommentOutput {
    id: String,
    author_display_name: Option<String>,
    body: Option<String>,
    created: String,
    updated: String,
}

/// Runs a complete Jira show operation behind the domain interface.
pub fn execute(client: &JiraClient, args: &ShowArgs) -> Result<ShowIssueOutput, AppError> {
    let prepared = prepare(args);
    let response = client
        .get_issue::<ShowIssueFields>(prepared.request())
        .map_err(|source| AppError::ExecuteShow { source })?;
    let url = client
        .issue_browse_url(&response.key)
        .map_err(|source| AppError::ExecuteShow { source })?;

    Ok(output_from_response(
        response,
        url,
        prepared.include_comments(),
    ))
}

/// Builds the Jira request without executing it; useful as the module's narrow test surface.
pub fn prepare(args: &ShowArgs) -> PreparedShowIssue {
    PreparedShowIssue {
        request: GetIssueRequest {
            issue_id_or_key: args.issue_key.clone(),
            fields: show_issue_fields(args.comments),
            expand: Vec::new(),
        },
        include_comments: args.comments,
    }
}

/// Human rendering is show-specific, while JSON rendering stays generic in `render`.
pub fn render_human(mut writer: impl Write, issue: &ShowIssueOutput) -> Result<(), AppError> {
    use render::ansi::{BOLD, CYAN, GREEN, RESET, YELLOW};

    writeln!(
        writer,
        "{BOLD}{CYAN}{}{RESET} {BOLD}{}{RESET}",
        issue.key, issue.summary
    )
    .map_err(|source| AppError::RenderOutput { source })?;
    writeln!(writer).map_err(|source| AppError::RenderOutput { source })?;

    write_field(&mut writer, "Status", &issue.status_name)?;
    write_field(&mut writer, "Type", &issue.issue_type_name)?;
    write_field(
        &mut writer,
        "Priority",
        issue.priority_name.as_deref().unwrap_or("Unprioritized"),
    )?;
    write_field(
        &mut writer,
        "Assignee",
        issue
            .assignee_display_name
            .as_deref()
            .unwrap_or("Unassigned"),
    )?;
    write_field(
        &mut writer,
        "Reporter",
        issue.reporter_display_name.as_deref().unwrap_or("Unknown"),
    )?;
    write_field(&mut writer, "Created", &issue.created)?;
    write_field(&mut writer, "Updated", &issue.updated)?;
    write_field(&mut writer, "Components", &display_list(&issue.components))?;
    write_field(&mut writer, "URL", &issue.url)?;

    writeln!(writer).map_err(|source| AppError::RenderOutput { source })?;
    writeln!(writer, "{BOLD}{GREEN}Description{RESET}")
        .map_err(|source| AppError::RenderOutput { source })?;
    writeln!(
        writer,
        "{}",
        issue.description.as_deref().unwrap_or("No description.")
    )
    .map_err(|source| AppError::RenderOutput { source })?;

    if !issue.comments.is_empty() {
        writeln!(writer).map_err(|source| AppError::RenderOutput { source })?;
        writeln!(writer, "{BOLD}{YELLOW}Comments{RESET}")
            .map_err(|source| AppError::RenderOutput { source })?;

        for comment in &issue.comments {
            writeln!(
                writer,
                "- {} ({})",
                comment.author_display_name.as_deref().unwrap_or("Unknown"),
                comment.created
            )
            .map_err(|source| AppError::RenderOutput { source })?;
            writeln!(writer, "  {}", comment.body.as_deref().unwrap_or(""))
                .map_err(|source| AppError::RenderOutput { source })?;
        }
    }

    Ok(())
}

impl PreparedShowIssue {
    /// Exposes only the transport request; show preparation remains inside this module.
    pub fn request(&self) -> &GetIssueRequest {
        &self.request
    }

    fn include_comments(&self) -> bool {
        self.include_comments
    }
}

// Fields are part of the show output contract, not a caller option.
fn show_issue_fields(include_comments: bool) -> Vec<String> {
    let mut fields = vec![
        "summary".to_string(),
        "status".to_string(),
        "components".to_string(),
        "issuetype".to_string(),
        "priority".to_string(),
        "assignee".to_string(),
        "reporter".to_string(),
        "created".to_string(),
        "updated".to_string(),
        "description".to_string(),
    ];

    if include_comments {
        fields.push("comment".to_string());
    }

    fields
}

fn output_from_response(
    response: GetIssueResponse<ShowIssueFields>,
    url: String,
    include_comments: bool,
) -> ShowIssueOutput {
    let comments = if include_comments {
        response
            .fields
            .comment
            .comments
            .into_iter()
            .map(|comment| CommentOutput {
                id: comment.id,
                author_display_name: comment.author.map(|author| author.display_name),
                body: comment.body.as_ref().and_then(adf_to_plain_text),
                created: comment.created,
                updated: comment.updated,
            })
            .collect()
    } else {
        Vec::new()
    };

    ShowIssueOutput {
        key: response.key,
        summary: response.fields.summary,
        status_name: response.fields.status.name,
        components: response
            .fields
            .components
            .into_iter()
            .map(|component| component.name)
            .collect(),
        issue_type_name: response.fields.issue_type.name,
        priority_name: response.fields.priority.map(|priority| priority.name),
        assignee_display_name: response
            .fields
            .assignee
            .map(|assignee| assignee.display_name),
        reporter_display_name: response
            .fields
            .reporter
            .map(|reporter| reporter.display_name),
        created: response.fields.created,
        updated: response.fields.updated,
        url,
        description: response
            .fields
            .description
            .as_ref()
            .and_then(adf_to_plain_text),
        comments,
    }
}

fn write_field(writer: &mut impl Write, label: &str, value: &str) -> Result<(), AppError> {
    use render::ansi::{BOLD, RESET};

    writeln!(writer, "{BOLD}{label}:{RESET} {value}")
        .map_err(|source| AppError::RenderOutput { source })
}

fn display_list(values: &[String]) -> String {
    if values.is_empty() {
        "None".to_string()
    } else {
        values.join(", ")
    }
}

fn adf_to_plain_text(value: &Value) -> Option<String> {
    let mut output = String::new();
    collect_adf_text(value, &mut output);
    let output = normalize_plain_text(&output);

    if output.is_empty() {
        None
    } else {
        Some(output)
    }
}

fn collect_adf_text(value: &Value, output: &mut String) {
    match value {
        Value::String(text) => output.push_str(text),
        Value::Array(items) => {
            for item in items {
                collect_adf_text(item, output);
            }
        }
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("text")
                && let Some(text) = object.get("text").and_then(Value::as_str)
            {
                output.push_str(text);
                return;
            }

            if object.get("type").and_then(Value::as_str) == Some("hardBreak") {
                output.push('\n');
                return;
            }

            if let Some(content) = object.get("content") {
                collect_adf_text(content, output);
                if matches!(
                    object.get("type").and_then(Value::as_str),
                    Some("paragraph" | "heading" | "listItem")
                ) {
                    output.push('\n');
                }
            }
        }
        _ => {}
    }
}

fn normalize_plain_text(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
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
    fn show_request_contains_expected_fields_without_comments() {
        let prepared = prepare(&ShowArgs {
            issue_key: "DEMO-101".to_string(),
            json: false,
            comments: false,
        });
        let request = prepared.request();

        assert_eq!(request.issue_id_or_key, "DEMO-101");
        assert!(!request.fields.contains(&"comment".to_string()));
        assert_eq!(request.fields.len(), 10);
    }

    #[test]
    fn show_request_includes_comments_when_requested() {
        let prepared = prepare(&ShowArgs {
            issue_key: "DEMO-101".to_string(),
            json: false,
            comments: true,
        });
        let request = prepared.request();

        assert!(request.fields.contains(&"comment".to_string()));
    }

    #[test]
    fn deserializes_realistic_show_fixture_into_output() {
        let response: GetIssueResponse<ShowIssueFields> =
            serde_json::from_str(&fixture("show-basic.json")).unwrap();

        let output = output_from_response(
            response,
            "https://example.atlassian.net/browse/DEMO-101".to_string(),
            true,
        );

        assert_eq!(output.key, "DEMO-101");
        assert_eq!(output.status_name, "In Review");
        assert_eq!(output.issue_type_name, "Task");
        assert_eq!(output.priority_name.as_deref(), Some("High"));
        assert_eq!(
            output.assignee_display_name.as_deref(),
            Some("Alex Example")
        );
        assert_eq!(output.components, vec!["Web Platform"]);
        assert_eq!(
            output.description.as_deref(),
            Some("Review and align the CSP configuration.")
        );
        assert_eq!(output.comments.len(), 1);
        assert_eq!(
            output.comments[0].body.as_deref(),
            Some("Looks good to me.")
        );
    }

    #[test]
    fn render_human_emits_colorized_issue_details() {
        let response: GetIssueResponse<ShowIssueFields> =
            serde_json::from_str(&fixture("show-basic.json")).unwrap();
        let output = output_from_response(
            response,
            "https://example.atlassian.net/browse/DEMO-101".to_string(),
            false,
        );
        let mut rendered = Vec::new();

        render_human(&mut rendered, &output).unwrap();
        let rendered = String::from_utf8(rendered).unwrap();

        assert!(rendered.contains("\u{1b}[1m\u{1b}[36mDEMO-101\u{1b}[0m"));
        assert!(rendered.contains("Status:\u{1b}[0m In Review"));
        assert!(rendered.contains("Description"));
        assert!(!rendered.contains("Comments"));
    }

    #[test]
    fn render_json_emits_stable_jeera_owned_schema() {
        let response: GetIssueResponse<ShowIssueFields> =
            serde_json::from_str(&fixture("show-basic.json")).unwrap();
        let output = output_from_response(
            response,
            "https://example.atlassian.net/browse/DEMO-101".to_string(),
            true,
        );
        let mut rendered = Vec::new();

        render::render_json(&mut rendered, &output).unwrap();

        assert!(
            String::from_utf8(rendered)
                .unwrap()
                .contains("\"comments\": [")
        );
    }
}
