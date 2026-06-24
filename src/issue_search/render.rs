use super::SearchContinuation;
use super::intent::{SearchColumn, SearchIntent, serialize_human_columns};
use super::output::{SearchIssueOutput, SearchOutput};
use crate::error::AppError;
use crate::jql::SortDirection;
use std::io::Write;

pub(super) fn render_human_output(
    mut writer: impl Write,
    output: &SearchOutput,
    columns: &[SearchColumn],
    next_page_command: Option<&str>,
) -> Result<(), AppError> {
    if output.issues().is_empty() {
        writeln!(writer, "No issues found.").map_err(|source| AppError::RenderOutput { source })?;
    } else if columns.is_empty() {
        for issue in output.issues() {
            let components = render_components(issue);

            writeln!(
                writer,
                "{} [{}] {}{}",
                render_key(&issue.key),
                render_status(&issue.status_name),
                issue.summary,
                if components.is_empty() {
                    String::new()
                } else {
                    format!(" ({components})")
                }
            )
            .map_err(|source| AppError::RenderOutput { source })?;
        }
    } else {
        for issue in output.issues() {
            let row = columns
                .iter()
                .map(|column| column.render(issue))
                .collect::<Vec<_>>()
                .join(" | ");
            writeln!(writer, "{row}").map_err(|source| AppError::RenderOutput { source })?;
        }
    }

    if !output.is_last()
        && let Some(next_page_token) = output.next_page_token()
    {
        writeln!(writer, "Next page token: {next_page_token}")
            .map_err(|source| AppError::RenderOutput { source })?;

        if let Some(next_page_command) = next_page_command {
            writeln!(writer, "Next page command: {next_page_command}")
                .map_err(|source| AppError::RenderOutput { source })?;
        }
    }

    Ok(())
}

pub(super) fn build_next_page_command(
    intent: &SearchIntent,
    continuation: &SearchContinuation,
    default_limit: u32,
) -> String {
    let mut parts = vec!["jeera".to_string(), "search".to_string()];

    if intent.json {
        parts.push("--json".to_string());
    }
    if let Some(jql) = &intent.jql {
        parts.push("--jql".to_string());
        parts.push(shell_quote(jql));
    }
    if let Some(board) = &intent.board {
        parts.push("--board".to_string());
        parts.push(shell_quote(&board.to_cli_value()));
    }
    if let Some(project) = &intent.project {
        parts.push("--project".to_string());
        parts.push(shell_quote(project));
    }
    if let Some(assignee) = &intent.assignee {
        parts.push("--assignee".to_string());
        parts.push(shell_quote(assignee));
    }
    if intent.unassigned {
        parts.push("--unassigned".to_string());
    }
    if let Some(reporter) = &intent.reporter {
        parts.push("--reporter".to_string());
        parts.push(shell_quote(reporter));
    }
    for status in &intent.status {
        parts.push("--status".to_string());
        parts.push(shell_quote(status));
    }
    if let Some(status_category) = &intent.status_category {
        parts.push("--status-category".to_string());
        parts.push(shell_quote(status_category));
    }
    for issue_type in &intent.issue_type {
        parts.push("--type".to_string());
        parts.push(shell_quote(issue_type));
    }
    for component in &intent.component {
        parts.push("--component".to_string());
        parts.push(shell_quote(component));
    }
    for label in &intent.label {
        parts.push("--label".to_string());
        parts.push(shell_quote(label));
    }
    if let Some(text) = &intent.text {
        parts.push("--text".to_string());
        parts.push(shell_quote(text));
    }
    if intent.open {
        parts.push("--open".to_string());
    }
    parts.push("--limit".to_string());
    parts.push(intent.limit.unwrap_or(default_limit).to_string());
    if let Some(columns) = serialize_human_columns(&intent.human_columns) {
        parts.push("--columns".to_string());
        parts.push(shell_quote(&columns));
    }
    if let Some(sort) = &intent.sort {
        parts.push("--sort".to_string());
        parts.push(shell_quote(sort));
    }
    if intent.sort_direction == Some(SortDirection::Asc) {
        parts.push("--asc".to_string());
    }
    if intent.sort_direction == Some(SortDirection::Desc) {
        parts.push("--desc".to_string());
    }
    parts.push("--next-page-token".to_string());
    parts.push(shell_quote(continuation.next_page_token()));
    if let Some(query) = &intent.query {
        parts.push(shell_quote(query));
    }

    parts.join(" ")
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value.bytes().all(|byte| {
            matches!(
                byte,
                b'a'..=b'z'
                    | b'A'..=b'Z'
                    | b'0'..=b'9'
                    | b'-'
                    | b'_'
                    | b'.'
                    | b'/'
                    | b':'
                    | b'@'
                    | b'='
            )
        })
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

pub(super) fn render_key(key: &str) -> String {
    use crate::render::ansi::{BOLD, CYAN, RESET};

    format!("{BOLD}{CYAN}{key}{RESET}")
}

pub(super) fn render_status(status_name: &str) -> String {
    use crate::render::ansi::{DIM, GREEN, RESET, YELLOW};

    let lowercase = status_name.to_ascii_lowercase();
    let color = if lowercase.contains("done")
        || lowercase.contains("closed")
        || lowercase.contains("resolved")
    {
        Some(GREEN)
    } else if lowercase.contains("progress")
        || lowercase.contains("review")
        || lowercase.contains("test")
        || lowercase.contains("qa")
        || lowercase.contains("blocked")
    {
        Some(YELLOW)
    } else if lowercase.contains("to do")
        || lowercase.contains("todo")
        || lowercase.contains("backlog")
        || lowercase.contains("selected")
        || lowercase.contains("open")
    {
        Some(DIM)
    } else {
        None
    };

    match color {
        Some(color) => format!("{color}{status_name}{RESET}"),
        None => status_name.to_string(),
    }
}

pub(super) fn render_components(issue: &SearchIssueOutput) -> String {
    use crate::render::ansi::{DIM, RESET};

    let components = issue.components.join(", ");
    if components.is_empty() {
        String::new()
    } else {
        format!("{DIM}{components}{RESET}")
    }
}
