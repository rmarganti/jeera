use super::board::{BoardJqlFilter, board_filter, resolve_board_id, resolve_board_name};
use super::intent::{SearchIntent, has_explicit_search_restriction, validate_search_intent};
use super::output::{SearchIssueFields, SearchOutput, output_from_search_response};
use super::query::{query_from_search_intent, search_fields};
use crate::client::{JiraClient, types::SearchIssuesRequest};
use crate::error::AppError;

/// Prepared search intent after validation, board resolution, JQL assembly, and field selection.
#[derive(Debug)]
pub(crate) struct PreparedIssueSearch {
    request: SearchIssuesRequest,
}

#[derive(Debug)]
pub struct SearchExecution {
    effective_intent: SearchIntent,
    final_jql: String,
    output: SearchOutput,
    continuation: Option<SearchContinuation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchContinuation {
    next_page_token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchOutputMode {
    json: bool,
}

impl SearchExecution {
    pub fn output(&self) -> &SearchOutput {
        &self.output
    }

    pub fn output_mode(&self) -> SearchOutputMode {
        SearchOutputMode {
            json: self.effective_intent.json,
        }
    }

    pub fn effective_intent(&self) -> &SearchIntent {
        &self.effective_intent
    }

    pub fn final_jql(&self) -> &str {
        &self.final_jql
    }

    pub fn should_debug_jql(&self) -> bool {
        self.effective_intent.debug_jql
    }

    pub fn continuation(&self) -> Option<&SearchContinuation> {
        self.continuation.as_ref()
    }

    pub(super) fn new(
        effective_intent: SearchIntent,
        final_jql: String,
        output: SearchOutput,
        continuation: Option<SearchContinuation>,
    ) -> Self {
        Self {
            effective_intent,
            final_jql,
            output,
            continuation,
        }
    }
}

impl SearchOutputMode {
    pub fn is_json(self) -> bool {
        self.json
    }
}

impl SearchContinuation {
    pub fn next_page_token(&self) -> &str {
        &self.next_page_token
    }

    pub(super) fn from_output(output: &SearchOutput) -> Option<Self> {
        (!output.is_last())
            .then(|| output.next_page_token().map(ToOwned::to_owned))
            .flatten()
            .map(|next_page_token| Self { next_page_token })
    }
}

impl PreparedIssueSearch {
    pub(crate) fn request(&self) -> &SearchIssuesRequest {
        &self.request
    }

    pub(crate) fn jql(&self) -> &str {
        &self.request.jql
    }
}

pub(super) fn execute_prepared(
    client: &JiraClient,
    prepared: &PreparedIssueSearch,
) -> Result<SearchOutput, AppError> {
    let response = client
        .search_issues::<SearchIssueFields>(prepared.request())
        .map_err(|source| AppError::ExecuteSearch { source })?;

    Ok(output_from_search_response(response))
}

pub(super) fn prepare(
    client: &JiraClient,
    intent: &SearchIntent,
    min_limit: u32,
    max_limit: u32,
    default_limit: u32,
) -> Result<PreparedIssueSearch, AppError> {
    prepare_with_board_source(
        intent,
        client.default_board_id(),
        |board_name| resolve_board_name(client, board_name),
        |board_id| board_filter(client, board_id),
        min_limit,
        max_limit,
        default_limit,
    )
}

// Internal seam for tests: production uses JiraClient, tests use closure adapters.
pub(super) fn prepare_with_board_source<R, F>(
    intent: &SearchIntent,
    default_board_id: Option<u64>,
    resolve_board_name: R,
    load_board_filter: F,
    min_limit: u32,
    max_limit: u32,
    default_limit: u32,
) -> Result<PreparedIssueSearch, AppError>
where
    R: FnMut(&str) -> Result<u64, AppError>,
    F: FnOnce(u64) -> Result<BoardJqlFilter, AppError>,
{
    validate_search_intent(intent, min_limit, max_limit, default_limit)?;

    let configured_board_id =
        resolve_board_id(intent.board.as_ref(), default_board_id, resolve_board_name)?;
    if configured_board_id.is_none() && !has_explicit_search_restriction(intent) {
        return Err(AppError::InvalidSearch {
            reason: "provide at least one search restriction, such as QUERY, --jql, --board, --project, --assignee, --component, --status, --label, --text, or configure default_board_id".to_string(),
        });
    }

    let board_filter = configured_board_id.map(load_board_filter).transpose()?;
    let jql = query_from_search_intent(intent, board_filter).to_jql();

    Ok(PreparedIssueSearch {
        request: SearchIssuesRequest {
            jql,
            next_page_token: intent.next_page_token.clone(),
            max_results: Some(intent.limit.unwrap_or(default_limit)),
            fields: search_fields(intent.json, &intent.human_columns),
            ..Default::default()
        },
    })
}
