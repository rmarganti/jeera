use crate::cli::{ShowCreateMetaArgs, ShowEditMetaArgs, ShowTransitionsArgs};
use crate::client::JiraClient;
use crate::{error::AppError, issue_metadata, render};
use std::io;

pub fn show_create_meta(client: &JiraClient, args: &ShowCreateMetaArgs) -> Result<(), AppError> {
    let output = issue_metadata::show_create_meta(client, args)?;

    if args.json {
        render::render_json(io::stdout().lock(), &output)?;
    } else {
        issue_metadata::render_create_meta_human(io::stdout().lock(), &output)?;
    }

    Ok(())
}

pub fn show_edit_meta(client: &JiraClient, args: &ShowEditMetaArgs) -> Result<(), AppError> {
    let output = issue_metadata::show_edit_meta(client, args)?;

    if args.json {
        render::render_json(io::stdout().lock(), &output)?;
    } else {
        issue_metadata::render_edit_meta_human(io::stdout().lock(), &output)?;
    }

    Ok(())
}

pub fn show_transitions(client: &JiraClient, args: &ShowTransitionsArgs) -> Result<(), AppError> {
    let output = issue_metadata::show_transitions(client, args)?;

    if args.json {
        render::render_json(io::stdout().lock(), &output)?;
    } else {
        issue_metadata::render_transitions_human(io::stdout().lock(), &output)?;
    }

    Ok(())
}
