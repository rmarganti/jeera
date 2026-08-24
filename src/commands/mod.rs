use crate::cli::Command;
use crate::client::JiraClient;
use crate::error::AppError;

pub mod boards;
pub mod create;
pub mod metadata;
pub mod search;
pub mod show;
pub mod update;

pub fn run(client: &JiraClient, command: Command, mutations_enabled: bool) -> Result<(), AppError> {
    if command.is_mutation() && !mutations_enabled {
        return Err(AppError::MutationsDisabled);
    }

    match command {
        Command::Boards(args) => boards::run(client, &args),
        Command::Search(args) => search::run(client, &args),
        Command::Show(args) => show::run(client, &args),
        Command::Create(args) => create::run(client, &args),
        Command::Update(args) => update::run(client, &args),
        Command::ShowCreateMeta(args) => metadata::show_create_meta(client, &args),
        Command::ShowEditMeta(args) => metadata::show_edit_meta(client, &args),
        Command::ShowTransitions(args) => metadata::show_transitions(client, &args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::UpdateArgs;
    use crate::client::{JiraAuth, JiraClientConfig};
    use std::collections::BTreeMap;
    use std::time::Duration;
    use url::Url;

    #[test]
    fn update_is_rejected_when_mutations_are_disabled() {
        let client = JiraClient::new(JiraClientConfig {
            base_url: Url::parse("https://example.atlassian.net/").unwrap(),
            auth: JiraAuth::Bearer {
                token: "secret".to_string(),
            },
            timeout: Duration::from_secs(30),
            default_board_id: None,
            searches: BTreeMap::new(),
        });
        let command = Command::Update(UpdateArgs {
            issue_key: "GCCDEV-1".to_string(),
            summary: Some("Updated".to_string()),
            ..Default::default()
        });

        let error = run(&client, command, false).unwrap_err();
        assert!(matches!(error, AppError::MutationsDisabled));
    }
}
