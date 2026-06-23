use crate::cli::ShowArgs;
use crate::client::JiraClient;
use crate::{error::AppError, issue_show, render};
use std::io;

// Thin command adapter: delegate show behavior to the domain module, choose output mode here.
pub fn run(client: &JiraClient, args: &ShowArgs) -> Result<(), AppError> {
    let output = issue_show::execute(client, args)?;

    if args.json {
        render::render_json(io::stdout().lock(), &output)?;
    } else {
        issue_show::render_human(io::stdout().lock(), &output)?;
    }

    Ok(())
}
