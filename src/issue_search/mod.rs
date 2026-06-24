//! Jira issue search domain module.

mod board;
mod intent;
mod output;
mod prepare;
mod profile;
mod query;
mod render;

use crate::client::JiraClient;
use crate::error::AppError;
use std::io::Write;

#[allow(unused_imports)]
pub use intent::{SearchColumn, SearchIntent};
#[allow(unused_imports)]
pub use output::SearchOutput;
#[allow(unused_imports)]
pub use prepare::{SearchContinuation, SearchExecution, SearchOutputMode};

use prepare::{execute_prepared, prepare};
use profile::merge_search_profile;
use render::{build_next_page_command, render_human_output};

const SEARCH_MIN_LIMIT: u32 = 1;
const SEARCH_MAX_LIMIT: u32 = 100;
const DEFAULT_SEARCH_LIMIT: u32 = 50;

/// Runs a complete Jira issue search behind the domain interface.
pub fn execute(client: &JiraClient, intent: SearchIntent) -> Result<SearchExecution, AppError> {
    let effective_intent = merge_search_profile(client, &intent)?;
    let prepared = prepare(
        client,
        &effective_intent,
        SEARCH_MIN_LIMIT,
        SEARCH_MAX_LIMIT,
        DEFAULT_SEARCH_LIMIT,
    )?;
    let output = execute_prepared(client, &prepared)?;
    let continuation = SearchContinuation::from_output(&output);

    Ok(SearchExecution::new(
        effective_intent,
        prepared.jql().to_string(),
        output,
        continuation,
    ))
}

/// Human rendering is search-specific, while JSON rendering stays generic in `render`.
pub fn render_human(mut writer: impl Write, execution: &SearchExecution) -> Result<(), AppError> {
    let effective_intent = execution.effective_intent();
    let next_page_command = execution.continuation().map(|continuation| {
        build_next_page_command(effective_intent, continuation, DEFAULT_SEARCH_LIMIT)
    });
    render_human_output(
        &mut writer,
        execution.output(),
        effective_intent.human_columns(),
        next_page_command.as_deref(),
    )
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_support;
