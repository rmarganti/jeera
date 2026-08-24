use crate::cli::UpdateArgs;
use crate::client::JiraClient;
use crate::{error::AppError, issue_update, render};
use std::io;

pub fn run(client: &JiraClient, args: &UpdateArgs) -> Result<(), AppError> {
    let output = issue_update::execute(client, args)?;

    if args.json {
        render::render_json(io::stdout().lock(), &output)?;
    } else {
        issue_update::render_human(io::stdout().lock(), &output)?;
    }

    Ok(())
}
