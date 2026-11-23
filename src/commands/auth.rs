use crate::cli::{AuthArgs, AuthCommand};
use crate::config;
use crate::error::AppError;
use crate::keychain;
use std::io::{self, IsTerminal, Read};

pub fn run(args: AuthArgs) -> Result<(), AppError> {
    match args.command {
        AuthCommand::SetToken => set_token(),
    }
}

fn set_token() -> Result<(), AppError> {
    let target = config::load_auth_target().map_err(|source| AppError::LoadConfig { source })?;
    let account = &target.keychain_account;

    let token = read_token(account).map_err(|source| AppError::ReadToken { source })?;
    let token = token.trim();
    if token.is_empty() {
        return Err(AppError::EmptyToken);
    }

    keychain::set_token(account, token).map_err(|source| AppError::StoreToken { source })?;

    eprintln!(
        "Stored Jira token in the macOS Keychain (service {:?}, account {account:?}).",
        keychain::SERVICE
    );
    if target.has_plaintext_token {
        eprintln!("Your settings.json still contains a plaintext token; remove it.");
    }

    Ok(())
}

/// Prompt without echo on a terminal; otherwise read stdin so the token can be piped in
/// without landing in shell history or process arguments.
fn read_token(account: &str) -> io::Result<String> {
    if io::stdin().is_terminal() {
        rpassword::prompt_password(format!("Jira API token for {account}: "))
    } else {
        let mut token = String::new();
        io::stdin().read_to_string(&mut token)?;
        Ok(token)
    }
}
