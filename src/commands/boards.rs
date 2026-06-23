use crate::cli::BoardsArgs;
use crate::client::JiraClient;
use crate::{error::AppError, issue_boards, render};
use std::io::{self, Write};

// Thin command adapter: delegate boards behavior to the domain module, choose output mode here.
pub fn run(client: &JiraClient, args: &BoardsArgs) -> Result<(), AppError> {
    run_with_writer(client, args, io::stdout().lock())
}

fn run_with_writer(
    client: &JiraClient,
    args: &BoardsArgs,
    mut stdout: impl Write,
) -> Result<(), AppError> {
    let output = issue_boards::execute(client, args)?;

    if args.json {
        render::render_json(&mut stdout, &output)?;
    } else {
        issue_boards::render_human(&mut stdout, &output)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{JiraAuth, JiraClient, JiraClientConfig};
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::thread;
    use std::time::Duration;
    use url::Url;

    fn fixture(path: &str) -> String {
        fs::read_to_string(Path::new("tests/fixtures/jira").join(path)).unwrap()
    }

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
    fn boards_runs_and_requests_project_filtered_listing() {
        let (base_url, rx) = spawn_server("200 OK", &fixture("boards-basic.json"));
        let client = test_client(base_url);
        let args = BoardsArgs {
            project: Some("GCCDEV".to_string()),
            json: false,
        };

        let mut stdout = Vec::new();

        run_with_writer(&client, &args, &mut stdout).unwrap();

        let request = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(request.starts_with("GET /rest/agile/1.0/board?projectKeyOrId=GCCDEV HTTP/1.1"));
    }
}
