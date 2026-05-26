mod cli;
mod client;
mod commands;
mod config;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();

    let settings = match config::Settings::load() {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("Config load failed: {error}");
            return;
        }
    };

    let jira_client_config = settings.into_jira_client_config();
    let client = client::JiraClient::new(jira_client_config);

    commands::run(&client, cli.command);
}
