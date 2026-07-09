use crate::cli::CreateArgs;
use crate::client::JiraClient;
use crate::{error::AppError, issue_create, render};
use std::io;

// Thin command adapter: delegate create behavior to the domain module, choose output mode here.
pub fn run(client: &JiraClient, args: &CreateArgs) -> Result<(), AppError> {
    let output = issue_create::execute(client, args)?;

    if args.json {
        render::render_json(io::stdout().lock(), &output)?;
    } else {
        issue_create::render_human(io::stdout().lock(), &output)?;
    }

    Ok(())
}
