use crate::cli::Command;
use crate::client::JiraClient;

pub mod search;

pub fn run(client: &JiraClient, command: Command) {
    match command {
        Command::Search(args) => search::run(client, &args),
    }
}
