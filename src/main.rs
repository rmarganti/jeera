mod board;
mod cli;
mod client;
mod commands;
mod config;
mod error;
mod issue_boards;
mod issue_create;
mod issue_fields;
mod issue_metadata;
mod issue_search;
mod issue_show;
mod issue_update;
mod jql;
mod render;

use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), error::AppError> {
    let cli = cli::Cli::parse_with_guidance();

    let settings =
        config::Settings::load().map_err(|source| error::AppError::LoadConfig { source })?;

    let mutations_enabled = settings.mutations_enabled;
    let jira_client_config = settings.into_jira_client_config();
    let client = client::JiraClient::new(jira_client_config);

    commands::run(&client, cli.command, mutations_enabled)
}
