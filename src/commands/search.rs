use crate::cli::SearchArgs;
use crate::client::JiraClient;
use crate::{error::AppError, issue_search, render};
use std::io::{self, Write};

// Thin command adapter: delegate search behavior to the domain module, choose output mode here.
pub fn run(client: &JiraClient, args: &SearchArgs) -> Result<(), AppError> {
    run_with_writers(client, args, io::stdout().lock(), io::stderr().lock())
}

fn run_with_writers(
    client: &JiraClient,
    args: &SearchArgs,
    mut stdout: impl Write,
    mut stderr: impl Write,
) -> Result<(), AppError> {
    let prepared = issue_search::prepare(client, args)?;

    if args.debug_jql {
        writeln!(stderr, "Final JQL: {}", prepared.jql())
            .map_err(|source| AppError::RenderOutput { source })?;
    }

    let output = issue_search::execute_prepared(client, &prepared)?;

    if args.json {
        render::render_json(&mut stdout, &output)?;
    } else {
        let next_page_command = output
            .next_page_token()
            .filter(|_| !output.is_last())
            .map(|next_page_token| build_next_page_command(args, next_page_token));
        issue_search::render_human(
            &mut stdout,
            &output,
            prepared.human_columns(),
            next_page_command.as_deref(),
        )?;
    }

    Ok(())
}

fn build_next_page_command(args: &SearchArgs, next_page_token: &str) -> String {
    let mut parts = vec!["jeera".to_string(), "search".to_string()];

    if args.json {
        parts.push("--json".to_string());
    }
    if let Some(jql) = &args.jql {
        parts.push("--jql".to_string());
        parts.push(shell_quote(jql));
    }
    if let Some(board) = &args.board {
        parts.push("--board".to_string());
        parts.push(shell_quote(board));
    }
    if let Some(project) = &args.project {
        parts.push("--project".to_string());
        parts.push(shell_quote(project));
    }
    if let Some(assignee) = &args.assignee {
        parts.push("--assignee".to_string());
        parts.push(shell_quote(assignee));
    }
    if args.unassigned {
        parts.push("--unassigned".to_string());
    }
    if let Some(reporter) = &args.reporter {
        parts.push("--reporter".to_string());
        parts.push(shell_quote(reporter));
    }
    for status in &args.status {
        parts.push("--status".to_string());
        parts.push(shell_quote(status));
    }
    if let Some(status_category) = &args.status_category {
        parts.push("--status-category".to_string());
        parts.push(shell_quote(status_category));
    }
    for issue_type in &args.issue_type {
        parts.push("--type".to_string());
        parts.push(shell_quote(issue_type));
    }
    for component in &args.component {
        parts.push("--component".to_string());
        parts.push(shell_quote(component));
    }
    for label in &args.label {
        parts.push("--label".to_string());
        parts.push(shell_quote(label));
    }
    if let Some(text) = &args.text {
        parts.push("--text".to_string());
        parts.push(shell_quote(text));
    }
    if args.open {
        parts.push("--open".to_string());
    }
    parts.push("--limit".to_string());
    parts.push(args.limit.to_string());
    if let Some(columns) = &args.columns {
        parts.push("--columns".to_string());
        parts.push(shell_quote(columns));
    }
    if let Some(sort) = &args.sort {
        parts.push("--sort".to_string());
        parts.push(shell_quote(sort));
    }
    if args.asc {
        parts.push("--asc".to_string());
    }
    if args.desc {
        parts.push("--desc".to_string());
    }
    parts.push("--next-page-token".to_string());
    parts.push(shell_quote(next_page_token));
    if let Some(query) = &args.query {
        parts.push(shell_quote(query));
    }

    parts.join(" ")
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b':' | b'@' | b'='))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::SearchArgs;
    use crate::client::{JiraAuth, JiraClient, JiraClientConfig};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;
    use url::Url;

    fn spawn_server(status_line: &str, body: &str) -> (Url, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let (tx, rx) = std::sync::mpsc::channel();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();

            let mut buffer = [0_u8; 8192];
            let bytes_read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..bytes_read]).into_owned();
            tx.send(request).unwrap();
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });

        (Url::parse(&format!("http://{addr}/")).unwrap(), rx)
    }

    fn test_client(base_url: Url) -> JiraClient {
        JiraClient::new(JiraClientConfig {
            base_url,
            auth: JiraAuth::Basic {
                email: "user@example.com".to_string(),
                api_token: "token".to_string(),
            },
            timeout: Duration::from_secs(5),
            default_board_id: None,
        })
    }

    #[test]
    fn debug_jql_prints_final_query_to_stderr_before_running_search() {
        let body = r#"{"isLast":true,"issues":[]}"#;
        let (base_url, rx) = spawn_server("200 OK", body);
        let client = test_client(base_url);
        let args = SearchArgs {
            assignee: Some("me".to_string()),
            debug_jql: true,
            ..Default::default()
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        run_with_writers(&client, &args, &mut stdout, &mut stderr).unwrap();

        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(String::from_utf8(stdout).unwrap(), "No issues found.\n");
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            "Final JQL: assignee = currentUser() ORDER BY updated DESC\n"
        );
    }

    #[test]
    fn build_next_page_command_preserves_filters_and_quotes_spaces() {
        let args = SearchArgs {
            query: Some("release blockers".to_string()),
            board: Some("215".to_string()),
            project: Some("GCCDEV".to_string()),
            status: vec!["In Progress".to_string()],
            component: vec!["Core Platform".to_string()],
            columns: Some("key,status,summary".to_string()),
            sort: Some("rank".to_string()),
            desc: true,
            limit: 1,
            ..Default::default()
        };

        assert_eq!(
            build_next_page_command(&args, "token with spaces"),
            "jeera search --board 215 --project GCCDEV --status 'In Progress' --component 'Core Platform' --limit 1 --columns 'key,status,summary' --sort rank --desc --next-page-token 'token with spaces' 'release blockers'"
        );
    }

    #[test]
    fn build_next_page_command_quotes_named_board_references() {
        let args = SearchArgs {
            board: Some("GCCDEV Kanban Board".to_string()),
            limit: 2,
            ..Default::default()
        };

        assert_eq!(
            build_next_page_command(&args, "next-token"),
            "jeera search --board 'GCCDEV Kanban Board' --limit 2 --next-page-token next-token"
        );
    }

    #[test]
    fn run_prints_copy_pasteable_next_page_command() {
        let body = r#"{"isLast":false,"nextPageToken":"next token","issues":[]}"#;
        let (base_url, rx) = spawn_server("200 OK", body);
        let client = test_client(base_url);
        let args = SearchArgs {
            project: Some("GCCDEV".to_string()),
            component: vec!["QQMS".to_string()],
            limit: 1,
            ..Default::default()
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        run_with_writers(&client, &args, &mut stdout, &mut stderr).unwrap();

        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(String::from_utf8(stderr).unwrap(), "");
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            "No issues found.\nNext page token: next token\nNext page command: jeera search --project GCCDEV --component QQMS --limit 1 --next-page-token 'next token'\n"
        );
    }
}
