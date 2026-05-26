use crate::client::types::{JiraError, SearchIssuesRequest, SearchIssuesResponse};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Serialize, de::DeserializeOwned};

pub mod types;

pub struct JiraClient {
    http: ureq::Agent,
    config: JiraClientConfig,
}

pub struct JiraClientConfig {
    pub base_url: String,
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

    pub fn search_issues(
        &self,
        request: &SearchIssuesRequest,
    ) -> Result<SearchIssuesResponse, JiraError> {
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
        let url = format!("{}{}", self.config.base_url, path);
        let mut builder = ureq::http::Request::builder()
            .method(method)
            .uri(&url)
            .header("Accept", "application/json")
            .header("Authorization", self.authorization_header());

        let response = match body {
            Some(payload) => {
                let payload = serde_json::to_vec(payload)
                    .map_err(|error| JiraError::Json(error.to_string()))?;
                builder = builder.header("Content-Type", "application/json");
                let request = builder
                    .body(payload)
                    .map_err(|error| JiraError::Http(error.to_string()))?;
                self.http
                    .run(request)
                    .map_err(|error| JiraError::Http(error.to_string()))?
            }
            None => {
                let request = builder
                    .body(())
                    .map_err(|error| JiraError::Http(error.to_string()))?;
                self.http
                    .run(request)
                    .map_err(|error| JiraError::Http(error.to_string()))?
            }
        };

        let status = response.status().as_u16();
        let body = response
            .into_body()
            .read_to_string()
            .map_err(|error| JiraError::Http(error.to_string()))?;

        if !(200..300).contains(&status) {
            return Err(JiraError::HttpStatus { status, body });
        }

        serde_json::from_str(&body).map_err(|error| JiraError::Json(error.to_string()))
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
