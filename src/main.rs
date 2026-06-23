mod cli;
mod client;
mod commands;
mod config;
mod error;
mod issue_search;
mod jql;
mod render;

use clap::Parser;
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
    let cli = cli::Cli::parse();

    let settings =
        config::Settings::load().map_err(|source| error::AppError::LoadConfig { source })?;

    let jira_client_config = settings.into_jira_client_config();
    let client = client::JiraClient::new(jira_client_config);

    commands::run(&client, cli.command)
}
