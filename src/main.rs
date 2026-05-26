mod client;
mod commands;
mod config;

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

    commands::search::run(&client);
}
