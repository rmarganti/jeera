mod client;
mod config;
mod core;

fn main() {
    let settings = match config::Settings::load() {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("Config load failed: {error}");
            return;
        }
    };

    let jira_client_config = settings.into_jira_client_config();
    let client = client::JiraClient::new(jira_client_config);

    let search_request = crate::client::types::SearchIssuesRequest {
        jql: "assignee = currentUser() ORDER BY updated DESC".to_string(),
        max_results: Some(5),
        fields: vec![
            "summary".to_string(),
            "status".to_string(),
            "components".to_string(),
        ],
        ..Default::default()
    };

    match client.search_issues(&search_request) {
        Ok(response) => {
            println!("{:#?}", response)
        }
        Err(error) => eprintln!("Search failed: {error}"),
    }
}
