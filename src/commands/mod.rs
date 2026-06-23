use crate::cli::Command;
use crate::client::JiraClient;
use crate::error::AppError;

pub mod search;
pub mod show;

pub fn run(client: &JiraClient, command: Command) -> Result<(), AppError> {
    match command {
        Command::Search(args) => search::run(client, &args),
        Command::Show(args) => show::run(client, &args),
    }
}
