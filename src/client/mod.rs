use crate::client::types::{JiraError, SearchIssuesRequest, SearchIssuesResponse};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Serialize, de::DeserializeOwned};
use url::Url;

pub mod types;

pub struct JiraClient {
    http: ureq::Agent,
    config: JiraClientConfig,
}

pub struct JiraClientConfig {
    pub base_url: Url,
    pub auth: JiraAuth,
}

pub enum JiraAuth {
    Basic { email: String, api_token: String },
    Bearer { token: String },
}

impl JiraClient {
    pub fn new(config: JiraClientConfig) -> Self {
        let http_config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build();

        JiraClient {
            config,
            http: http_config.into(),
        }
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
            "/rest/api/3/search/jql",
            Some(request),
        )
    }

    /// Helper method to send JSON requests and handle responses.
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
            .join(path)
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
            return Err(JiraError::HttpStatus { status, body });
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
        JiraClient::new(JiraClientConfig {
            base_url: Url::parse(base_url).unwrap(),
            auth,
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
        assert_eq!(body["jql"], "project = DEMO ORDER BY updated DESC");
        assert_eq!(body["maxResults"], 2);
        assert_eq!(body["fields"], serde_json::json!(["summary", "status"]));
    }

    #[test]
    fn search_issues_returns_http_status_errors_with_body() {
        let (base_url, _rx) = spawn_server(
            "401 Unauthorized",
            r#"{"errorMessages":["Unauthorized"]}"#.to_string(),
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
            JiraError::HttpStatus { status, body } => {
                assert_eq!(status, 401);
                assert!(body.contains("Unauthorized"));
            }
            other => panic!("expected HttpStatus error, got {other:?}"),
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
