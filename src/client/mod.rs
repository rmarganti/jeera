use crate::client::types::{
    GetBoardConfigurationRequest, GetBoardConfigurationResponse, GetIssueRequest, GetIssueResponse,
    JiraError, JiraErrorResponse, ListBoardsRequest, ListBoardsResponse, SearchIssuesRequest,
    SearchIssuesResponse,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Serialize, de::DeserializeOwned};
use std::time::Duration;
use url::Url;

pub mod types;

pub struct JiraClient {
    http: ureq::Agent,
    config: JiraClientConfig,
}

pub struct JiraClientConfig {
    pub base_url: Url,
    pub auth: JiraAuth,
    pub timeout: Duration,
    pub default_board_id: Option<u64>,
}

pub enum JiraAuth {
    Basic { email: String, api_token: String },
    Bearer { token: String },
}

impl JiraClient {
    pub fn new(config: JiraClientConfig) -> Self {
        let http_config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .user_agent(client_user_agent())
            .timeout_global(Some(config.timeout))
            .build();

        JiraClient {
            config,
            http: http_config.into(),
        }
    }

    pub fn get_issue<F>(&self, request: &GetIssueRequest) -> Result<GetIssueResponse<F>, JiraError>
    where
        F: DeserializeOwned,
    {
        let mut path = format!("rest/api/3/issue/{}", request.issue_id_or_key);
        let mut query = Vec::new();

        if !request.fields.is_empty() {
            query.push(format!("fields={}", request.fields.join(",")));
        }

        if !request.expand.is_empty() {
            query.push(format!("expand={}", request.expand.join(",")));
        }

        if !query.is_empty() {
            path.push('?');
            path.push_str(&query.join("&"));
        }

        self.send_json::<(), GetIssueResponse<F>>(ureq::http::Method::GET, &path, None)
    }

    pub fn default_board_id(&self) -> Option<u64> {
        self.config.default_board_id
    }

    pub fn list_boards(
        &self,
        request: &ListBoardsRequest,
    ) -> Result<ListBoardsResponse, JiraError> {
        let mut path = "rest/agile/1.0/board".to_string();

        if let Some(project_key_or_id) = &request.project_key_or_id {
            let query = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("projectKeyOrId", project_key_or_id)
                .finish();
            path.push('?');
            path.push_str(&query);
        }

        self.send_json::<(), ListBoardsResponse>(ureq::http::Method::GET, &path, None)
    }

    pub fn issue_browse_url(&self, issue_key: &str) -> Result<String, JiraError> {
        self.config
            .base_url
            .join(&format!("browse/{issue_key}"))
            .map(|url| url.to_string())
            .map_err(|source| JiraError::BuildUrl { source })
    }

    pub fn search_issues<F>(
        &self,
        request: &SearchIssuesRequest,
    ) -> Result<SearchIssuesResponse<F>, JiraError>
    where
        F: DeserializeOwned,
    {
        self.send_json(
            ureq::http::Method::POST,
            "rest/api/3/search/jql",
            Some(request),
        )
    }

    pub fn get_board_configuration(
        &self,
        request: &GetBoardConfigurationRequest,
    ) -> Result<GetBoardConfigurationResponse, JiraError> {
        let path = format!("rest/agile/1.0/board/{}/configuration", request.board_id);

        self.send_json::<(), GetBoardConfigurationResponse>(ureq::http::Method::GET, &path, None)
    }

    fn send_json<Request, Response>(
        &self,
        method: ureq::http::Method,
        path: &str,
        body: Option<&Request>,
    ) -> Result<Response, JiraError>
    where
        Request: Serialize,
        Response: DeserializeOwned,
    {
        let url = self
            .config
            .base_url
            .join(path.trim_start_matches('/'))
            .map_err(|source| JiraError::BuildUrl { source })?;

        let mut builder = ureq::http::Request::builder()
            .method(method)
            .uri(url.as_str())
            .header("Accept", "application/json")
            .header("Authorization", self.authorization_header());

        let response = match body {
            Some(payload) => {
                let payload = serde_json::to_vec(payload)
                    .map_err(|source| JiraError::EncodeRequestBody { source })?;
                builder = builder.header("Content-Type", "application/json");
                let request = builder
                    .body(payload)
                    .map_err(|source| JiraError::BuildRequest { source })?;
                self.http
                    .run(request)
                    .map_err(|source| JiraError::Transport { source })?
            }
            None => {
                let request = builder
                    .body(())
                    .map_err(|source| JiraError::BuildRequest { source })?;
                self.http
                    .run(request)
                    .map_err(|source| JiraError::Transport { source })?
            }
        };

        let status = response.status().as_u16();
        let body = response
            .into_body()
            .read_to_string()
            .map_err(|source| JiraError::ReadResponseBody { source })?;

        if !(200..300).contains(&status) {
            let message = summarize_http_error_body(&body);
            return Err(JiraError::HttpStatus {
                status,
                endpoint: format!("/{}", path.trim_start_matches('/')),
                message,
            });
        }

        serde_json::from_str(&body).map_err(|source| JiraError::DecodeResponse { source })
    }

    fn authorization_header(&self) -> String {
        match &self.config.auth {
            JiraAuth::Basic { email, api_token } => {
                let credentials = STANDARD.encode(format!("{email}:{api_token}"));
                format!("Basic {credentials}")
            }
            JiraAuth::Bearer { token } => format!("Bearer {token}"),
        }
    }
}

fn client_user_agent() -> String {
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    format!("jeera/{} ({build})", env!("CARGO_PKG_VERSION"))
}

fn summarize_http_error_body(body: &str) -> String {
    if let Ok(error) = serde_json::from_str::<JiraErrorResponse>(body)
        && let Some(message) = error.summary()
    {
        return message;
    }

    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "response body was empty".to_string();
    }

    collapse_and_truncate(trimmed, 240)
}

fn collapse_and_truncate(body: &str, max_chars: usize) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        collapsed
    } else {
        let mut excerpt = collapsed.chars().take(max_chars).collect::<String>();
        excerpt.push('…');
        excerpt
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::sync::mpsc;
    use std::thread;

    struct CapturedRequest {
        method: String,
        path: String,
        headers: String,
        body: String,
    }

    fn fixture(path: &str) -> String {
        fs::read_to_string(Path::new("tests/fixtures/jira").join(path)).unwrap()
    }

    fn client(base_url: &str, auth: JiraAuth) -> JiraClient {
        client_with_timeout(base_url, auth, Duration::from_secs(30))
    }

    fn client_with_timeout(base_url: &str, auth: JiraAuth, timeout: Duration) -> JiraClient {
        JiraClient::new(JiraClientConfig {
            base_url: Url::parse(base_url).unwrap(),
            auth,
            timeout,
            default_board_id: None,
        })
    }

    fn spawn_server(
        response_status: &str,
        response_body: String,
    ) -> (String, mpsc::Receiver<CapturedRequest>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        let response_status = response_status.to_string();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = Vec::new();
            let mut temp = [0_u8; 1024];
            let header_end;

            loop {
                let read = stream.read(&mut temp).unwrap();
                if read == 0 {
                    return;
                }
                buffer.extend_from_slice(&temp[..read]);
                if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                    header_end = index + 4;
                    break;
                }
            }

            let header_text = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
            let mut lines = header_text.split("\r\n");
            let request_line = lines.next().unwrap();
            let mut request_line_parts = request_line.split_whitespace();
            let method = request_line_parts.next().unwrap().to_string();
            let path = request_line_parts.next().unwrap().to_string();
            let headers = lines
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n");

            let content_length = header_text
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then_some(value)
                })
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);

            let mut body = buffer[header_end..].to_vec();
            while body.len() < content_length {
                let read = stream.read(&mut temp).unwrap();
                if read == 0 {
                    break;
                }
                body.extend_from_slice(&temp[..read]);
            }

            tx.send(CapturedRequest {
                method,
                path,
                headers,
                body: String::from_utf8_lossy(&body[..content_length]).into_owned(),
            })
            .unwrap();

            let response = format!(
                "HTTP/1.1 {response_status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        (address, rx)
    }

    fn spawn_delayed_server(delay: Duration, response_body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = Vec::new();
            let mut temp = [0_u8; 1024];

            loop {
                let read = stream.read(&mut temp).unwrap();
                if read == 0 {
                    return;
                }
                buffer.extend_from_slice(&temp[..read]);
                if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }

            thread::sleep(delay);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        address
    }

    #[test]
    fn builds_basic_authorization_header() {
        let client = client(
            "https://example.atlassian.net/",
            JiraAuth::Basic {
                email: "you@example.com".to_string(),
                api_token: "secret-token".to_string(),
            },
        );

        assert_eq!(
            client.authorization_header(),
            "Basic eW91QGV4YW1wbGUuY29tOnNlY3JldC10b2tlbg=="
        );
    }

    #[test]
    fn builds_bearer_authorization_header() {
        let client = client(
            "https://example.atlassian.net/",
            JiraAuth::Bearer {
                token: "secret-token".to_string(),
            },
        );

        assert_eq!(client.authorization_header(), "Bearer secret-token");
    }

    #[test]
    fn search_issues_sends_expected_request() {
        let (base_url, rx) = spawn_server("200 OK", fixture("search-basic.json"));
        let client = client(
            &base_url,
            JiraAuth::Bearer {
                token: "secret-token".to_string(),
            },
        );
        let request = SearchIssuesRequest {
            jql: "project = DEMO ORDER BY updated DESC".to_string(),
            max_results: Some(2),
            fields: vec!["summary".to_string(), "status".to_string()],
            ..Default::default()
        };

        let _: SearchIssuesResponse<Value> = client.search_issues(&request).unwrap();
        let captured = rx.recv().unwrap();
        let body: Value = serde_json::from_str(&captured.body).unwrap();
        let headers = captured.headers.to_ascii_lowercase();

        assert_eq!(captured.method, "POST");
        assert_eq!(captured.path, "/rest/api/3/search/jql");
        assert!(headers.contains("accept: application/json"));
        assert!(headers.contains("content-type: application/json"));
        assert!(headers.contains("authorization: bearer secret-token"));
        assert!(headers.contains("user-agent: jeera/"));
        assert_eq!(body["jql"], "project = DEMO ORDER BY updated DESC");
        assert_eq!(body["maxResults"], 2);
        assert_eq!(body["fields"], serde_json::json!(["summary", "status"]));
    }

    #[test]
    fn list_boards_sends_expected_request() {
        let (base_url, rx) = spawn_server("200 OK", fixture("boards-basic.json"));
        let client = client(
            &base_url,
            JiraAuth::Bearer {
                token: "secret-token".to_string(),
            },
        );
        let request = ListBoardsRequest {
            project_key_or_id: Some("GCCDEV".to_string()),
        };

        let response = client.list_boards(&request).unwrap();
        let captured = rx.recv().unwrap();
        let headers = captured.headers.to_ascii_lowercase();

        assert_eq!(captured.method, "GET");
        assert_eq!(captured.path, "/rest/agile/1.0/board?projectKeyOrId=GCCDEV");
        assert!(headers.contains("accept: application/json"));
        assert!(headers.contains("authorization: bearer secret-token"));
        assert!(captured.body.is_empty());
        assert_eq!(response.values.len(), 4);
        assert_eq!(response.values[0].id, 215);
        assert_eq!(response.values[0].board_type, "kanban");
        assert_eq!(
            response.values[0]
                .location
                .as_ref()
                .and_then(|l| l.project_key.as_deref()),
            Some("GCCDEV")
        );
    }

    #[test]
    fn get_board_configuration_sends_expected_request() {
        let (base_url, rx) = spawn_server(
            "200 OK",
            r#"{"filter":{"id":"10492"},"subQuery":{"query":"fixVersion is EMPTY"}}"#.to_string(),
        );
        let client = client(
            &base_url,
            JiraAuth::Bearer {
                token: "secret-token".to_string(),
            },
        );
        let request = GetBoardConfigurationRequest { board_id: 215 };

        let response = client.get_board_configuration(&request).unwrap();
        let captured = rx.recv().unwrap();
        let headers = captured.headers.to_ascii_lowercase();

        assert_eq!(captured.method, "GET");
        assert_eq!(captured.path, "/rest/agile/1.0/board/215/configuration");
        assert!(headers.contains("accept: application/json"));
        assert!(headers.contains("authorization: bearer secret-token"));
        assert!(captured.body.is_empty());
        assert_eq!(response.filter.id, "10492");
        assert_eq!(response.sub_query.query, "fixVersion is EMPTY");
    }

    #[test]
    fn get_issue_sends_expected_request() {
        let (base_url, rx) = spawn_server("200 OK", fixture("show-basic.json"));
        let client = client(
            &base_url,
            JiraAuth::Bearer {
                token: "secret-token".to_string(),
            },
        );
        let request = GetIssueRequest {
            issue_id_or_key: "DEMO-101".to_string(),
            fields: vec!["summary".to_string(), "status".to_string()],
            expand: vec!["renderedFields".to_string()],
        };

        let _: GetIssueResponse<Value> = client.get_issue(&request).unwrap();
        let captured = rx.recv().unwrap();
        let headers = captured.headers.to_ascii_lowercase();

        assert_eq!(captured.method, "GET");
        assert_eq!(
            captured.path,
            "/rest/api/3/issue/DEMO-101?fields=summary,status&expand=renderedFields"
        );
        assert!(headers.contains("accept: application/json"));
        assert!(headers.contains("authorization: bearer secret-token"));
        assert!(headers.contains("user-agent: jeera/"));
        assert!(captured.body.is_empty());
    }

    #[test]
    fn issue_browse_url_uses_configured_base_url() {
        let client = client(
            "https://example.atlassian.net/jira/",
            JiraAuth::Bearer {
                token: "secret-token".to_string(),
            },
        );

        assert_eq!(
            client.issue_browse_url("DEMO-101").unwrap(),
            "https://example.atlassian.net/jira/browse/DEMO-101"
        );
    }

    #[test]
    fn search_issues_formats_jira_error_body() {
        let (base_url, _rx) = spawn_server(
            "401 Unauthorized",
            r#"{"errorMessages":["Unauthorized"],"errors":{"projectKey":"Project key is required"}}"#
                .to_string(),
        );
        let client = client(
            &base_url,
            JiraAuth::Bearer {
                token: "secret-token".to_string(),
            },
        );

        let error = client
            .search_issues::<Value>(&SearchIssuesRequest {
                jql: "project = DEMO".to_string(),
                ..Default::default()
            })
            .unwrap_err();

        match error {
            JiraError::HttpStatus {
                status,
                endpoint,
                message,
            } => {
                assert_eq!(status, 401);
                assert_eq!(endpoint, "/rest/api/3/search/jql");
                assert_eq!(message, "Unauthorized; projectKey: Project key is required");
            }
            other => panic!("expected HttpStatus error, got {other:?}"),
        }
    }

    #[test]
    fn search_issues_falls_back_to_raw_http_error_body() {
        let (base_url, _rx) = spawn_server("500 Internal Server Error", "oops".to_string());
        let client = client(
            &base_url,
            JiraAuth::Bearer {
                token: "secret-token".to_string(),
            },
        );

        let error = client
            .search_issues::<Value>(&SearchIssuesRequest {
                jql: "project = DEMO".to_string(),
                ..Default::default()
            })
            .unwrap_err();

        match error {
            JiraError::HttpStatus {
                status, message, ..
            } => {
                assert_eq!(status, 500);
                assert_eq!(message, "oops");
            }
            other => panic!("expected HttpStatus error, got {other:?}"),
        }
    }

    #[test]
    fn search_issues_times_out_using_configured_timeout() {
        let base_url =
            spawn_delayed_server(Duration::from_millis(250), fixture("search-basic.json"));
        let client = client_with_timeout(
            &base_url,
            JiraAuth::Bearer {
                token: "secret-token".to_string(),
            },
            Duration::from_millis(50),
        );

        let error = client
            .search_issues::<Value>(&SearchIssuesRequest {
                jql: "project = DEMO".to_string(),
                ..Default::default()
            })
            .unwrap_err();

        match error {
            JiraError::Transport { source } => {
                assert!(source.to_string().to_lowercase().contains("timeout"));
            }
            other => panic!("expected Transport timeout error, got {other:?}"),
        }
    }

    #[test]
    fn search_issues_returns_decode_errors_for_invalid_json() {
        let (base_url, _rx) = spawn_server("200 OK", "not-json".to_string());
        let client = client(
            &base_url,
            JiraAuth::Bearer {
                token: "secret-token".to_string(),
            },
        );

        let error = client
            .search_issues::<Value>(&SearchIssuesRequest {
                jql: "project = DEMO".to_string(),
                ..Default::default()
            })
            .unwrap_err();

        assert!(matches!(error, JiraError::DecodeResponse { .. }));
    }
}
