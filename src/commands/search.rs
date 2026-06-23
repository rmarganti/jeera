use crate::cli::SearchArgs;
use crate::client::JiraClient;
use crate::{error::AppError, issue_search, render};
use std::io;

// Thin command adapter: delegate search behavior to the domain module, choose output mode here.
pub fn run(client: &JiraClient, args: &SearchArgs) -> Result<(), AppError> {
    let output = issue_search::execute(client, args)?;

    if args.json {
        render::render_json(io::stdout().lock(), &output)?;
    } else {
        issue_search::render_human(io::stdout().lock(), &output)?;
    }

    Ok(())
}
