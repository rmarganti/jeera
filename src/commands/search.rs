use crate::cli::SearchArgs;
use crate::client::JiraClient;
use crate::{error::AppError, issue_search, render};
use std::convert::TryFrom;
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
    let intent = issue_search::SearchIntent::try_from(args)?;
    let execution = issue_search::execute(client, intent)?;

    if execution.should_debug_jql() {
        writeln!(stderr, "Final JQL: {}", execution.final_jql())
            .map_err(|source| AppError::RenderOutput { source })?;
    }

    if execution.output_mode().is_json() {
        render::render_json(&mut stdout, execution.output())?;
    } else {
        issue_search::render_human(&mut stdout, &execution)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::SearchArgs;
    use crate::client::{JiraAuth, JiraClient, JiraClientConfig};
    use std::collections::BTreeMap;
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
            searches: BTreeMap::new(),
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
    fn run_prints_copy_pasteable_next_page_command() {
        let body = r#"{"isLast":false,"nextPageToken":"next token","issues":[]}"#;
        let (base_url, rx) = spawn_server("200 OK", body);
        let client = test_client(base_url);
        let args = SearchArgs {
            project: Some("SAMPLE".to_string()),
            component: vec!["QQMS".to_string()],
            limit: Some(1),
            ..Default::default()
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        run_with_writers(&client, &args, &mut stdout, &mut stderr).unwrap();

        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(String::from_utf8(stderr).unwrap(), "");
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            "No issues found.\nNext page token: next token\nNext page command: jeera search --project SAMPLE --component QQMS --limit 1 --next-page-token 'next token'\n"
        );
    }
}
