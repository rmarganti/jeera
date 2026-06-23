//! Jira issue search domain module.
//!
//! The command adapter enters through `prepare` and `execute_prepared`; tests and future
//! callers can use `prepare` when they only need the prepared Jira request.

use crate::cli::SearchArgs;
use crate::client::{
    JiraClient,
    types::{
        BoardResponse, GetBoardConfigurationRequest, ListBoardsRequest, SearchIssuesRequest,
        SearchIssuesResponse,
    },
};
use crate::error::AppError;
use crate::jql::{self, Clause, Order, Query, SortDirection, UserRef, Value};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::io::Write;

const SEARCH_MIN_LIMIT: u32 = 1;
const SEARCH_MAX_LIMIT: u32 = 100;

/// Prepared search intent after validation, board resolution, JQL assembly, and field selection.
#[derive(Debug)]
pub struct PreparedIssueSearch {
    request: SearchIssuesRequest,
    human_columns: HumanColumns,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchColumn {
    Key,
    Status,
    Summary,
    Components,
    Type,
    Assignee,
    Priority,
    Updated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HumanColumns {
    Default,
    Custom(Vec<SearchColumn>),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueStatus {
    name: String,
}

#[derive(Debug, Deserialize)]
struct IssueComponent {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NamedField {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserField {
    display_name: String,
}

// Jira response shape requested by `search_fields`; keep these in lockstep.
#[derive(Debug, Deserialize)]
struct SearchIssueFields {
    summary: String,
    status: IssueStatus,
    #[serde(default)]
    components: Vec<IssueComponent>,
    #[serde(rename = "issuetype")]
    issue_type: Option<NamedField>,
    priority: Option<NamedField>,
    assignee: Option<UserField>,
    updated: Option<String>,
}

// Domain form of Jira board configuration, before it becomes JQL clauses.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BoardJqlFilter {
    filter_id: u64,
    sub_query: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BoardSelector {
    Id(u64),
    Name(String),
}

/// jeera-owned search output; this is the stable interface for JSON and human rendering.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SearchOutput {
    issues: Vec<SearchIssueOutput>,
    is_last: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_page_token: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct SearchIssueOutput {
    key: String,
    summary: String,
    status_name: String,
    components: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    issue_type_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    assignee_display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    priority_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated: Option<String>,
}

/// Executes a previously prepared Jira issue search.
pub fn execute_prepared(
    client: &JiraClient,
    prepared: &PreparedIssueSearch,
) -> Result<SearchOutput, AppError> {
    let response = client
        .search_issues::<SearchIssueFields>(prepared.request())
        .map_err(|source| AppError::ExecuteSearch { source })?;

    Ok(output_from_search_response(response))
}

/// Builds the Jira request without executing it; useful as the module's narrow test surface.
pub fn prepare(client: &JiraClient, args: &SearchArgs) -> Result<PreparedIssueSearch, AppError> {
    prepare_with_board_source(
        args,
        client.default_board_id(),
        |board_name| resolve_board_name(client, board_name),
        |board_id| board_filter(client, board_id),
    )
}

/// Human rendering is search-specific, while JSON rendering stays generic in `render`.
pub(crate) fn render_human(
    mut writer: impl Write,
    output: &SearchOutput,
    columns: &[SearchColumn],
    next_page_command: Option<&str>,
) -> Result<(), AppError> {
    if output.issues.is_empty() {
        writeln!(writer, "No issues found.").map_err(|source| AppError::RenderOutput { source })?;
    } else if columns.is_empty() {
        for issue in &output.issues {
            let components = render_components(issue);

            writeln!(
                writer,
                "{} [{}] {}{}",
                render_key(&issue.key),
                render_status(&issue.status_name),
                issue.summary,
                if components.is_empty() {
                    String::new()
                } else {
                    format!(" ({components})")
                }
            )
            .map_err(|source| AppError::RenderOutput { source })?;
        }
    } else {
        for issue in &output.issues {
            let row = columns
                .iter()
                .map(|column| column.render(issue))
                .collect::<Vec<_>>()
                .join(" | ");
            writeln!(writer, "{row}").map_err(|source| AppError::RenderOutput { source })?;
        }
    }

    if !output.is_last
        && let Some(next_page_token) = &output.next_page_token
    {
        writeln!(writer, "Next page token: {next_page_token}")
            .map_err(|source| AppError::RenderOutput { source })?;

        if let Some(next_page_command) = next_page_command {
            writeln!(writer, "Next page command: {next_page_command}")
                .map_err(|source| AppError::RenderOutput { source })?;
        }
    }

    Ok(())
}

impl SearchOutput {
    pub(crate) fn is_last(&self) -> bool {
        self.is_last
    }

    pub(crate) fn next_page_token(&self) -> Option<&str> {
        self.next_page_token.as_deref()
    }
}

impl PreparedIssueSearch {
    /// Exposes only the transport request; search preparation remains inside this module.
    pub fn request(&self) -> &SearchIssuesRequest {
        &self.request
    }

    pub fn jql(&self) -> &str {
        &self.request.jql
    }

    pub(crate) fn human_columns(&self) -> &[SearchColumn] {
        match &self.human_columns {
            HumanColumns::Default => &[],
            HumanColumns::Custom(columns) => columns,
        }
    }
}

impl SearchColumn {
    fn parse(value: &str) -> Result<Self, AppError> {
        match value.trim() {
            "key" => Ok(Self::Key),
            "status" => Ok(Self::Status),
            "summary" => Ok(Self::Summary),
            "components" => Ok(Self::Components),
            "type" => Ok(Self::Type),
            "assignee" => Ok(Self::Assignee),
            "priority" => Ok(Self::Priority),
            "updated" => Ok(Self::Updated),
            "" => Err(AppError::InvalidSearch {
                reason: "--columns cannot contain empty values".to_string(),
            }),
            other => Err(AppError::InvalidSearch {
                reason: format!(
                    "unsupported --columns value {other:?}; expected one of key,status,summary,components,type,assignee,priority,updated"
                ),
            }),
        }
    }

    fn jira_field(self) -> Option<&'static str> {
        match self {
            Self::Key => None,
            Self::Status => Some("status"),
            Self::Summary => Some("summary"),
            Self::Components => Some("components"),
            Self::Type => Some("issuetype"),
            Self::Assignee => Some("assignee"),
            Self::Priority => Some("priority"),
            Self::Updated => Some("updated"),
        }
    }

    fn render(self, issue: &SearchIssueOutput) -> String {
        match self {
            Self::Key => render_key(&issue.key),
            Self::Status => render_status(&issue.status_name),
            Self::Summary => issue.summary.clone(),
            Self::Components => {
                if issue.components.is_empty() {
                    "-".to_string()
                } else {
                    render_components(issue)
                }
            }
            Self::Type => issue
                .issue_type_name
                .clone()
                .unwrap_or_else(|| "-".to_string()),
            Self::Assignee => issue
                .assignee_display_name
                .clone()
                .unwrap_or_else(|| "Unassigned".to_string()),
            Self::Priority => issue
                .priority_name
                .clone()
                .unwrap_or_else(|| "Unprioritized".to_string()),
            Self::Updated => issue.updated.clone().unwrap_or_else(|| "-".to_string()),
        }
    }
}

fn render_key(key: &str) -> String {
    use crate::render::ansi::{BOLD, CYAN, RESET};

    format!("{BOLD}{CYAN}{key}{RESET}")
}

fn render_status(status_name: &str) -> String {
    use crate::render::ansi::{DIM, GREEN, RESET, YELLOW};

    let lowercase = status_name.to_ascii_lowercase();
    let color = if lowercase.contains("done")
        || lowercase.contains("closed")
        || lowercase.contains("resolved")
    {
        Some(GREEN)
    } else if lowercase.contains("progress")
        || lowercase.contains("review")
        || lowercase.contains("test")
        || lowercase.contains("qa")
        || lowercase.contains("blocked")
    {
        Some(YELLOW)
    } else if lowercase.contains("to do")
        || lowercase.contains("todo")
        || lowercase.contains("backlog")
        || lowercase.contains("selected")
        || lowercase.contains("open")
    {
        Some(DIM)
    } else {
        None
    };

    match color {
        Some(color) => format!("{color}{status_name}{RESET}"),
        None => status_name.to_string(),
    }
}

fn render_components(issue: &SearchIssueOutput) -> String {
    use crate::render::ansi::{DIM, RESET};

    let components = issue.components.join(", ");
    if components.is_empty() {
        String::new()
    } else {
        format!("{DIM}{components}{RESET}")
    }
}

// Internal seam for tests: production uses JiraClient, tests use closure adapters.
fn prepare_with_board_source<R, F>(
    args: &SearchArgs,
    default_board_id: Option<u64>,
    resolve_board_name: R,
    load_board_filter: F,
) -> Result<PreparedIssueSearch, AppError>
where
    R: FnMut(&str) -> Result<u64, AppError>,
    F: FnOnce(u64) -> Result<BoardJqlFilter, AppError>,
{
    let human_columns = parse_human_columns(args.columns.as_deref())?;
    validate_search_args(args)?;

    let configured_board_id =
        resolve_board_id(args.board.as_deref(), default_board_id, resolve_board_name)?;
    if configured_board_id.is_none() && !has_explicit_search_restriction(args) {
        return Err(AppError::InvalidSearch {
            reason: "provide at least one search restriction, such as QUERY, --jql, --board, --project, --assignee, --component, --status, --label, --text, or configure default_board_id".to_string(),
        });
    }

    let board_filter = configured_board_id.map(load_board_filter).transpose()?;
    let jql = query_from_search_args(args, board_filter).to_jql();

    Ok(PreparedIssueSearch {
        request: SearchIssuesRequest {
            jql,
            next_page_token: args.next_page_token.clone(),
            max_results: Some(args.limit),
            fields: search_fields(args.json, &human_columns),
            ..Default::default()
        },
        human_columns,
    })
}

fn parse_human_columns(value: Option<&str>) -> Result<HumanColumns, AppError> {
    let Some(value) = value else {
        return Ok(HumanColumns::Default);
    };

    let columns = value
        .split(',')
        .map(SearchColumn::parse)
        .collect::<Result<Vec<_>, _>>()?;

    if columns.is_empty() {
        return Err(AppError::InvalidSearch {
            reason: "--columns cannot be empty".to_string(),
        });
    }

    let mut unique = Vec::new();
    for column in columns {
        if !unique.contains(&column) {
            unique.push(column);
        }
    }

    Ok(HumanColumns::Custom(unique))
}

fn has_explicit_search_restriction(args: &SearchArgs) -> bool {
    args.query
        .as_deref()
        .is_some_and(|query| !query.trim().is_empty())
        || args
            .jql
            .as_deref()
            .is_some_and(|jql| !jql.trim().is_empty())
        || args.project.is_some()
        || args.assignee.is_some()
        || args.unassigned
        || args.reporter.is_some()
        || !args.status.is_empty()
        || args.status_category.is_some()
        || !args.issue_type.is_empty()
        || !args.component.is_empty()
        || !args.label.is_empty()
        || args.text.is_some()
        || args.open
}

fn validate_search_args(args: &SearchArgs) -> Result<(), AppError> {
    validate_limit(args.limit)?;
    validate_optional_value("query", args.query.as_deref())?;
    validate_optional_value("jql", args.jql.as_deref())?;
    validate_optional_value("board", args.board.as_deref())?;
    validate_optional_value("project", args.project.as_deref())?;
    validate_optional_value("assignee", args.assignee.as_deref())?;
    validate_optional_value("reporter", args.reporter.as_deref())?;
    validate_optional_value("status-category", args.status_category.as_deref())?;
    validate_optional_value("text", args.text.as_deref())?;
    validate_optional_value("next-page-token", args.next_page_token.as_deref())?;
    validate_optional_value("columns", args.columns.as_deref())?;
    validate_repeated_values("status", &args.status)?;
    validate_repeated_values("type", &args.issue_type)?;
    validate_repeated_values("component", &args.component)?;
    validate_repeated_values("label", &args.label)?;
    if let Some(sort) = args.sort.as_deref() {
        validate_sort_field(sort)?;
    }
    Ok(())
}

fn validate_limit(limit: u32) -> Result<(), AppError> {
    if (SEARCH_MIN_LIMIT..=SEARCH_MAX_LIMIT).contains(&limit) {
        Ok(())
    } else {
        Err(AppError::InvalidSearch {
            reason: format!("--limit must be between {SEARCH_MIN_LIMIT} and {SEARCH_MAX_LIMIT}"),
        })
    }
}

fn validate_optional_value(field: &str, value: Option<&str>) -> Result<(), AppError> {
    match value {
        Some(value) if value.trim().is_empty() => Err(AppError::InvalidSearch {
            reason: format!("--{field} cannot be empty"),
        }),
        _ => Ok(()),
    }
}

fn validate_repeated_values(field: &str, values: &[String]) -> Result<(), AppError> {
    if values.iter().any(|value| value.trim().is_empty()) {
        Err(AppError::InvalidSearch {
            reason: format!("--{field} cannot contain empty values"),
        })
    } else {
        Ok(())
    }
}

fn validate_sort_field(sort: &str) -> Result<(), AppError> {
    if sort.trim().is_empty() {
        return Err(AppError::InvalidSearch {
            reason: "--sort cannot be empty".to_string(),
        });
    }

    if sort.contains(',') || sort.chars().any(char::is_whitespace) {
        return Err(AppError::InvalidSearch {
            reason:
                "--sort must be a single Jira field name; use --jql for custom ORDER BY clauses"
                    .to_string(),
        });
    }

    Ok(())
}

fn resolve_board_id<R>(
    board: Option<&str>,
    default_board_id: Option<u64>,
    mut resolve_board_name: R,
) -> Result<Option<u64>, AppError>
where
    R: FnMut(&str) -> Result<u64, AppError>,
{
    match board.map(str::trim) {
        Some(board) => match parse_board_selector(board)? {
            BoardSelector::Id(board_id) => Ok(Some(board_id)),
            BoardSelector::Name(board_name) => resolve_board_name(&board_name).map(Some),
        },
        None => Ok(default_board_id),
    }
}

fn parse_board_selector(board: &str) -> Result<BoardSelector, AppError> {
    if board.is_empty() {
        return Err(AppError::InvalidSearch {
            reason: "--board cannot be empty".to_string(),
        });
    }

    match board.parse::<u64>() {
        Ok(board_id) => Ok(BoardSelector::Id(board_id)),
        Err(_) => Ok(BoardSelector::Name(board.to_string())),
    }
}

fn resolve_board_name(client: &JiraClient, board_name: &str) -> Result<u64, AppError> {
    let response = client
        .list_boards(&ListBoardsRequest::default())
        .map_err(|source| AppError::ExecuteBoards { source })?;

    find_board_id_by_name(&response.values, board_name)
}

fn find_board_id_by_name(boards: &[BoardResponse], board_name: &str) -> Result<u64, AppError> {
    let exact_matches = boards
        .iter()
        .filter(|board| board.name == board_name)
        .collect::<Vec<_>>();
    let matches = if exact_matches.is_empty() {
        boards
            .iter()
            .filter(|board| board.name.eq_ignore_ascii_case(board_name))
            .collect::<Vec<_>>()
    } else {
        exact_matches
    };

    match matches.as_slice() {
        [] => Err(AppError::InvalidSearch {
            reason: format!(
                "no Jira board named {board_name:?} found; try `jeera boards` to discover available boards or pass a numeric --board ID"
            ),
        }),
        [board] => Ok(board.id),
        boards => Err(AppError::InvalidSearch {
            reason: format!(
                "board name {board_name:?} is ambiguous; matching board ids: {}. Try `jeera boards` or pass a numeric --board ID",
                boards
                    .iter()
                    .map(|board| board.id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }),
    }
}

fn board_filter(client: &JiraClient, board_id: u64) -> Result<BoardJqlFilter, AppError> {
    let configuration = client
        .get_board_configuration(&GetBoardConfigurationRequest { board_id })
        .map_err(|source| AppError::PrepareBoardSearch { board_id, source })?;

    Ok(BoardJqlFilter {
        filter_id: parse_board_filter_id(board_id, &configuration.filter.id)?,
        sub_query: Some(configuration.sub_query.query),
    })
}

fn parse_board_filter_id(board_id: u64, filter_id: &str) -> Result<u64, AppError> {
    filter_id
        .parse()
        .map_err(|source| AppError::InvalidBoardFilterId {
            board_id,
            filter_id: filter_id.to_string(),
            source,
        })
}

// Translates CLI-shaped search intent into generic JQL clauses.
fn query_from_search_args(args: &SearchArgs, board_filter: Option<BoardJqlFilter>) -> Query {
    let board_scoped = board_filter.is_some();
    let (raw_clause, raw_order_by) = args
        .jql
        .as_deref()
        .map(jql::split_order_by)
        .unwrap_or_default();
    let mut query = Query::new();

    if let Some(raw_clause) = raw_clause
        && !raw_clause.trim().is_empty()
    {
        query.push(Clause::raw(raw_clause.trim()));
    }

    if let Some(board_filter) = board_filter {
        query.push(Clause::field_equals(
            "filter",
            Value::number(board_filter.filter_id),
        ));
        if let Some(sub_query) = board_filter.sub_query
            && !sub_query.trim().is_empty()
        {
            query.push(Clause::raw(sub_query));
        }
    }

    if let Some(project) = &args.project {
        query.push(Clause::field_equals("project", Value::text(project)));
    }

    if args.unassigned {
        query.push(Clause::is_empty("assignee"));
    } else if let Some(assignee) = &args.assignee {
        query.push(Clause::field_equals(
            "assignee",
            UserRef::parse(assignee).to_value(),
        ));
    }

    if let Some(reporter) = &args.reporter {
        query.push(Clause::field_equals(
            "reporter",
            UserRef::parse(reporter).to_value(),
        ));
    }

    if !args.status.is_empty() {
        query.push(Clause::field_in("status", args.status.clone()));
    }

    if let Some(status_category) = &args.status_category {
        query.push(Clause::field_equals(
            "statusCategory",
            Value::text(status_category),
        ));
    }

    if !args.issue_type.is_empty() {
        query.push(Clause::field_in("issuetype", args.issue_type.clone()));
    }

    if !args.component.is_empty() {
        query.push(Clause::field_in("component", args.component.clone()));
    }

    if !args.label.is_empty() {
        query.push(Clause::field_in("labels", args.label.clone()));
    }

    if let Some(query_text) = &args.query {
        query.push(Clause::field_matches("text", query_text));
    }

    if let Some(text) = &args.text {
        query.push(Clause::field_matches("text", text));
    }

    if args.open {
        query.push(Clause::raw("statusCategory != Done"));
    }

    let order_by = raw_order_by
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(|| default_order(args, board_scoped), Order::raw);

    query.order_by(order_by);
    query
}

fn default_order(args: &SearchArgs, board_scoped: bool) -> Order {
    let field = args
        .sort
        .as_deref()
        .map(canonical_sort_field)
        .unwrap_or_else(|| {
            if board_scoped {
                Cow::Borrowed("Rank")
            } else {
                Cow::Borrowed("updated")
            }
        });

    let direction = if args.asc {
        SortDirection::Asc
    } else if args.desc {
        SortDirection::Desc
    } else if field.eq_ignore_ascii_case("Rank") {
        SortDirection::Asc
    } else {
        SortDirection::Desc
    };

    Order::field(field.into_owned(), direction)
}

fn canonical_sort_field(field: &str) -> Cow<'_, str> {
    if field.eq_ignore_ascii_case("rank") {
        Cow::Borrowed("Rank")
    } else if field.eq_ignore_ascii_case("updated") {
        Cow::Borrowed("updated")
    } else if field.eq_ignore_ascii_case("created") {
        Cow::Borrowed("created")
    } else if field.eq_ignore_ascii_case("priority") {
        Cow::Borrowed("priority")
    } else {
        Cow::Borrowed(field)
    }
}

fn search_fields(json: bool, human_columns: &HumanColumns) -> Vec<String> {
    let mut fields = vec![
        "summary".to_string(),
        "status".to_string(),
        "components".to_string(),
    ];

    let extra_columns: Vec<SearchColumn> = if json {
        vec![
            SearchColumn::Type,
            SearchColumn::Assignee,
            SearchColumn::Priority,
            SearchColumn::Updated,
        ]
    } else {
        match human_columns {
            HumanColumns::Default => Vec::new(),
            HumanColumns::Custom(columns) => columns.clone(),
        }
    };

    for column in extra_columns {
        if let Some(field) = column.jira_field()
            && !fields.iter().any(|existing| existing == field)
        {
            fields.push(field.to_string());
        }
    }

    fields
}

fn output_from_search_response(response: SearchIssuesResponse<SearchIssueFields>) -> SearchOutput {
    SearchOutput {
        issues: response
            .issues
            .into_iter()
            .map(SearchIssueOutput::from_issue)
            .collect(),
        is_last: response.is_last,
        next_page_token: response.next_page_token,
    }
}

impl SearchIssueOutput {
    fn from_issue(issue: crate::client::types::IssueResponse<SearchIssueFields>) -> Self {
        Self {
            key: issue.key,
            summary: issue.fields.summary,
            status_name: issue.fields.status.name,
            components: issue
                .fields
                .components
                .into_iter()
                .map(|component| component.name)
                .collect(),
            issue_type_name: issue.fields.issue_type.map(|issue_type| issue_type.name),
            assignee_display_name: issue.fields.assignee.map(|assignee| assignee.display_name),
            priority_name: issue.fields.priority.map(|priority| priority.name),
            updated: issue.fields.updated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::types::SearchIssuesResponse;
    use crate::render;
    use std::fs;
    use std::path::Path;

    fn fixture(path: &str) -> String {
        fs::read_to_string(Path::new("tests/fixtures/jira").join(path)).unwrap()
    }

    fn prepare_without_board(args: &SearchArgs) -> PreparedIssueSearch {
        prepare_with_board_source(args, None, |_| unreachable!(), |_| unreachable!()).unwrap()
    }

    #[test]
    fn search_requires_an_explicit_or_configured_restriction() {
        let error = prepare_with_board_source(
            &SearchArgs::default(),
            None,
            |_| unreachable!(),
            |_| unreachable!(),
        )
        .unwrap_err();

        assert!(matches!(error, AppError::InvalidSearch { .. }));
    }

    #[test]
    fn search_rejects_zero_limit() {
        let error = prepare_with_board_source(
            &SearchArgs {
                assignee: Some("me".to_string()),
                limit: 0,
                ..Default::default()
            },
            None,
            |_| unreachable!(),
            |_| unreachable!(),
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid search: --limit must be between 1 and 100"
        );
    }

    #[test]
    fn search_rejects_overly_large_limit() {
        let error = prepare_with_board_source(
            &SearchArgs {
                assignee: Some("me".to_string()),
                limit: 101,
                ..Default::default()
            },
            None,
            |_| unreachable!(),
            |_| unreachable!(),
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid search: --limit must be between 1 and 100"
        );
    }

    #[test]
    fn search_rejects_empty_string_filters() {
        for args in [
            SearchArgs {
                board: Some("   ".to_string()),
                ..Default::default()
            },
            SearchArgs {
                project: Some("   ".to_string()),
                ..Default::default()
            },
            SearchArgs {
                assignee: Some("".to_string()),
                ..Default::default()
            },
            SearchArgs {
                reporter: Some(" ".to_string()),
                ..Default::default()
            },
            SearchArgs {
                status_category: Some("\t".to_string()),
                ..Default::default()
            },
            SearchArgs {
                query: Some(" ".to_string()),
                ..Default::default()
            },
            SearchArgs {
                text: Some("".to_string()),
                ..Default::default()
            },
            SearchArgs {
                jql: Some("\n".to_string()),
                ..Default::default()
            },
            SearchArgs {
                next_page_token: Some(" ".to_string()),
                assignee: Some("me".to_string()),
                ..Default::default()
            },
            SearchArgs {
                columns: Some(" ".to_string()),
                assignee: Some("me".to_string()),
                ..Default::default()
            },
        ] {
            let error =
                prepare_with_board_source(&args, None, |_| unreachable!(), |_| unreachable!())
                    .unwrap_err();
            assert!(error.to_string().contains("cannot"));
        }
    }

    #[test]
    fn search_rejects_invalid_columns() {
        for columns in ["key,,summary", "key,unknown"] {
            let error = prepare_with_board_source(
                &SearchArgs {
                    assignee: Some("me".to_string()),
                    columns: Some(columns.to_string()),
                    ..Default::default()
                },
                None,
                |_| unreachable!(),
                |_| unreachable!(),
            )
            .unwrap_err();

            assert!(error.to_string().contains("--columns"));
        }
    }

    #[test]
    fn search_rejects_empty_values_in_multi_value_filters() {
        for args in [
            SearchArgs {
                status: vec!["In Progress".to_string(), " ".to_string()],
                ..Default::default()
            },
            SearchArgs {
                issue_type: vec!["Bug".to_string(), "".to_string()],
                ..Default::default()
            },
            SearchArgs {
                component: vec!["QQMS".to_string(), "\t".to_string()],
                ..Default::default()
            },
            SearchArgs {
                label: vec!["customer".to_string(), " ".to_string()],
                ..Default::default()
            },
        ] {
            let error =
                prepare_with_board_source(&args, None, |_| unreachable!(), |_| unreachable!())
                    .unwrap_err();
            assert!(error.to_string().contains("cannot contain empty values"));
        }
    }

    #[test]
    fn search_rejects_invalid_sort_values() {
        for sort in ["", "   ", "updated desc", "updated,created"] {
            let error = prepare_with_board_source(
                &SearchArgs {
                    assignee: Some("me".to_string()),
                    sort: Some(sort.to_string()),
                    ..Default::default()
                },
                None,
                |_| unreachable!(),
                |_| unreachable!(),
            )
            .unwrap_err();

            assert!(error.to_string().contains("--sort"));
        }
    }

    #[test]
    fn numeric_board_reference_bypasses_name_resolution() {
        let prepared = prepare_with_board_source(
            &SearchArgs {
                board: Some("215".to_string()),
                ..Default::default()
            },
            None,
            |_| panic!("numeric board ids should not invoke board-name resolution"),
            |board_id| {
                assert_eq!(board_id, 215);
                Ok(BoardJqlFilter {
                    filter_id: 10492,
                    sub_query: Some("fixVersion is EMPTY".to_string()),
                })
            },
        )
        .unwrap();

        assert_eq!(
            prepared.jql(),
            "filter = 10492 AND (fixVersion is EMPTY) ORDER BY Rank ASC"
        );
    }

    #[test]
    fn named_board_reference_resolves_before_loading_board_filter() {
        let prepared = prepare_with_board_source(
            &SearchArgs {
                board: Some("GCCDEV Kanban Board".to_string()),
                component: vec!["QQMS".to_string()],
                ..Default::default()
            },
            None,
            |board_name| {
                assert_eq!(board_name, "GCCDEV Kanban Board");
                Ok(215)
            },
            |board_id| {
                assert_eq!(board_id, 215);
                Ok(BoardJqlFilter {
                    filter_id: 10492,
                    sub_query: Some("fixVersion is EMPTY".to_string()),
                })
            },
        )
        .unwrap();

        assert_eq!(
            prepared.jql(),
            "filter = 10492 AND (fixVersion is EMPTY) AND component = \"QQMS\" ORDER BY Rank ASC"
        );
    }

    #[test]
    fn board_name_matching_is_case_insensitive_when_needed() {
        let boards = vec![BoardResponse {
            id: 215,
            name: "GCCDEV Kanban Board".to_string(),
            board_type: "kanban".to_string(),
            location: None,
        }];

        assert_eq!(
            find_board_id_by_name(&boards, "gccdev kanban board").unwrap(),
            215
        );
    }

    #[test]
    fn unknown_board_name_is_reported_clearly() {
        let boards = vec![BoardResponse {
            id: 215,
            name: "GCCDEV Kanban Board".to_string(),
            board_type: "kanban".to_string(),
            location: None,
        }];

        let error = find_board_id_by_name(&boards, "Missing Board").unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid search: no Jira board named \"Missing Board\" found; try `jeera boards` to discover available boards or pass a numeric --board ID"
        );
    }

    #[test]
    fn ambiguous_board_name_is_reported_clearly() {
        let boards = vec![
            BoardResponse {
                id: 215,
                name: "Team Board".to_string(),
                board_type: "kanban".to_string(),
                location: None,
            },
            BoardResponse {
                id: 314,
                name: "Team Board".to_string(),
                board_type: "scrum".to_string(),
                location: None,
            },
        ];

        let error = find_board_id_by_name(&boards, "Team Board").unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid search: board name \"Team Board\" is ambiguous; matching board ids: 215, 314. Try `jeera boards` or pass a numeric --board ID"
        );
    }

    #[test]
    fn invalid_board_filter_id_is_reported_instead_of_falling_back_to_board_id() {
        let error = parse_board_filter_id(215, "not-a-filter-id").unwrap_err();

        assert!(matches!(
            error,
            AppError::InvalidBoardFilterId {
                board_id: 215,
                filter_id,
                ..
            } if filter_id == "not-a-filter-id"
        ));
    }

    #[test]
    fn positional_query_is_an_explicit_search_restriction() {
        let prepared = prepare_without_board(&SearchArgs {
            query: Some("reporting".to_string()),
            ..Default::default()
        });

        assert_eq!(
            prepared.request().jql,
            "text ~ \"reporting\" ORDER BY updated DESC"
        );
    }

    #[test]
    fn positional_query_combines_with_default_board_filter() {
        let prepared = prepare_with_board_source(
            &SearchArgs {
                query: Some("reporting".to_string()),
                ..Default::default()
            },
            Some(215),
            |_| unreachable!(),
            |board_id| {
                assert_eq!(board_id, 215);
                Ok(BoardJqlFilter {
                    filter_id: 10492,
                    sub_query: Some("fixVersion is EMPTY".to_string()),
                })
            },
        )
        .unwrap();

        assert_eq!(
            prepared.request().jql,
            "filter = 10492 AND (fixVersion is EMPTY) AND text ~ \"reporting\" ORDER BY Rank ASC"
        );
    }

    #[test]
    fn positional_query_combines_with_raw_jql() {
        let prepared = prepare_without_board(&SearchArgs {
            query: Some("reporting".to_string()),
            jql: Some("project = GCCDEV ORDER BY Rank ASC".to_string()),
            ..Default::default()
        });

        assert_eq!(
            prepared.request().jql,
            "(project = GCCDEV) AND text ~ \"reporting\" ORDER BY Rank ASC"
        );
    }

    #[test]
    fn positional_query_and_text_flag_are_combined_with_and() {
        let prepared = prepare_without_board(&SearchArgs {
            query: Some("reporting".to_string()),
            text: Some("billing".to_string()),
            ..Default::default()
        });

        assert_eq!(
            prepared.request().jql,
            "text ~ \"reporting\" AND text ~ \"billing\" ORDER BY updated DESC"
        );
    }

    #[test]
    fn search_without_board_defaults_to_updated_desc() {
        let prepared = prepare_without_board(&SearchArgs {
            assignee: Some("me".to_string()),
            ..Default::default()
        });

        assert_eq!(
            prepared.jql(),
            "assignee = currentUser() ORDER BY updated DESC"
        );
    }

    #[test]
    fn board_search_defaults_to_rank_asc() {
        let prepared = prepare_with_board_source(
            &SearchArgs {
                component: vec!["QQMS".to_string()],
                ..Default::default()
            },
            Some(215),
            |_| unreachable!(),
            |board_id| {
                assert_eq!(board_id, 215);
                Ok(BoardJqlFilter {
                    filter_id: 10492,
                    sub_query: Some("fixVersion is EMPTY".to_string()),
                })
            },
        )
        .unwrap();

        assert_eq!(
            prepared.jql(),
            "filter = 10492 AND (fixVersion is EMPTY) AND component = \"QQMS\" ORDER BY Rank ASC"
        );
    }

    #[test]
    fn rank_sort_alias_maps_to_rank_asc_without_explicit_direction() {
        let prepared = prepare_without_board(&SearchArgs {
            assignee: Some("me".to_string()),
            sort: Some("rank".to_string()),
            ..Default::default()
        });

        assert_eq!(prepared.jql(), "assignee = currentUser() ORDER BY Rank ASC");
    }

    #[test]
    fn explicit_sort_still_defaults_to_desc_for_non_rank_fields() {
        let prepared = prepare_with_board_source(
            &SearchArgs {
                component: vec!["QQMS".to_string()],
                sort: Some("updated".to_string()),
                ..Default::default()
            },
            Some(215),
            |_| unreachable!(),
            |board_id| {
                assert_eq!(board_id, 215);
                Ok(BoardJqlFilter {
                    filter_id: 10492,
                    sub_query: Some("fixVersion is EMPTY".to_string()),
                })
            },
        )
        .unwrap();

        assert_eq!(
            prepared.jql(),
            "filter = 10492 AND (fixVersion is EMPTY) AND component = \"QQMS\" ORDER BY updated DESC"
        );
    }

    #[test]
    fn search_request_contains_expected_fields() {
        let args = SearchArgs {
            assignee: Some("me".to_string()),
            ..Default::default()
        };
        let prepared = prepare_without_board(&args);
        let request = prepared.request();

        assert_eq!(
            prepared.jql(),
            "assignee = currentUser() ORDER BY updated DESC"
        );
        assert_eq!(
            request.jql,
            "assignee = currentUser() ORDER BY updated DESC"
        );
        assert_eq!(request.max_results, Some(50));
        assert_eq!(request.fields, vec!["summary", "status", "components"]);
    }

    #[test]
    fn search_request_fetches_only_selected_extra_human_columns() {
        let prepared = prepare_without_board(&SearchArgs {
            assignee: Some("me".to_string()),
            columns: Some("key,type,status,assignee,updated,summary".to_string()),
            ..Default::default()
        });

        assert_eq!(
            prepared.request().fields,
            vec![
                "summary",
                "status",
                "components",
                "issuetype",
                "assignee",
                "updated"
            ]
        );
    }

    #[test]
    fn search_json_request_fetches_all_supported_columns_consistently() {
        let prepared = prepare_without_board(&SearchArgs {
            assignee: Some("me".to_string()),
            json: true,
            columns: Some("key,priority".to_string()),
            ..Default::default()
        });

        assert_eq!(
            prepared.request().fields,
            vec![
                "summary",
                "status",
                "components",
                "issuetype",
                "assignee",
                "priority",
                "updated"
            ]
        );
    }

    #[test]
    fn search_request_uses_pagination_args() {
        let args = SearchArgs {
            assignee: Some("me".to_string()),
            limit: 25,
            next_page_token: Some("token-123".to_string()),
            ..Default::default()
        };
        let prepared = prepare_without_board(&args);
        let request = prepared.request();

        assert_eq!(request.max_results, Some(25));
        assert_eq!(request.next_page_token, Some("token-123".to_string()));
    }

    #[test]
    fn explicit_desc_keeps_updated_desc_for_non_board_searches() {
        let prepared = prepare_without_board(&SearchArgs {
            assignee: Some("me".to_string()),
            desc: true,
            ..Default::default()
        });

        assert_eq!(
            prepared.request().jql,
            "assignee = currentUser() ORDER BY updated DESC"
        );
    }

    #[test]
    fn explicit_desc_flips_board_default_rank_sort_to_desc() {
        let prepared = prepare_with_board_source(
            &SearchArgs {
                component: vec!["QQMS".to_string()],
                desc: true,
                ..Default::default()
            },
            Some(215),
            |_| unreachable!(),
            |board_id| {
                assert_eq!(board_id, 215);
                Ok(BoardJqlFilter {
                    filter_id: 10492,
                    sub_query: Some("fixVersion is EMPTY".to_string()),
                })
            },
        )
        .unwrap();

        assert_eq!(
            prepared.request().jql,
            "filter = 10492 AND (fixVersion is EMPTY) AND component = \"QQMS\" ORDER BY Rank DESC"
        );
    }

    #[test]
    fn raw_jql_can_be_combined_with_explicit_filters() {
        let args = SearchArgs {
            jql: Some("project = GCCDEV ORDER BY Rank ASC".to_string()),
            component: vec!["QQMS".to_string()],
            ..Default::default()
        };
        let prepared = prepare_without_board(&args);

        assert_eq!(
            prepared.request().jql,
            "(project = GCCDEV) AND component = \"QQMS\" ORDER BY Rank ASC"
        );
    }

    #[test]
    fn board_filter_is_just_another_jql_clause() {
        let args = SearchArgs {
            component: vec!["QQMS".to_string()],
            ..Default::default()
        };
        let prepared = prepare_with_board_source(
            &args,
            Some(215),
            |_| unreachable!(),
            |board_id| {
                assert_eq!(board_id, 215);
                Ok(BoardJqlFilter {
                    filter_id: 10492,
                    sub_query: Some("fixVersion is EMPTY".to_string()),
                })
            },
        )
        .unwrap();

        assert_eq!(
            prepared.request().jql,
            "filter = 10492 AND (fixVersion is EMPTY) AND component = \"QQMS\" ORDER BY Rank ASC"
        );
    }

    #[test]
    fn final_jql_keeps_board_derived_clauses_when_combining_with_raw_jql() {
        let args = SearchArgs {
            jql: Some("project = GCCDEV ORDER BY Rank ASC".to_string()),
            component: vec!["QQMS".to_string()],
            ..Default::default()
        };
        let prepared = prepare_with_board_source(
            &args,
            Some(215),
            |_| unreachable!(),
            |board_id| {
                assert_eq!(board_id, 215);
                Ok(BoardJqlFilter {
                    filter_id: 10492,
                    sub_query: Some("fixVersion is EMPTY".to_string()),
                })
            },
        )
        .unwrap();

        assert_eq!(
            prepared.jql(),
            "(project = GCCDEV) AND filter = 10492 AND (fixVersion is EMPTY) AND component = \"QQMS\" ORDER BY Rank ASC"
        );
    }

    #[test]
    fn structured_filters_are_combined_and_values_are_escaped() {
        let args = SearchArgs {
            project: Some("GCCDEV".to_string()),
            assignee: Some("me".to_string()),
            status: vec!["In Progress".to_string(), "Ready \"Soon\"".to_string()],
            component: vec!["QQMS".to_string()],
            text: Some("reporting".to_string()),
            open: true,
            ..Default::default()
        };
        let prepared = prepare_without_board(&args);

        assert_eq!(
            prepared.request().jql,
            "project = \"GCCDEV\" AND assignee = currentUser() AND status in (\"In Progress\", \"Ready \\\"Soon\\\"\") AND component = \"QQMS\" AND text ~ \"reporting\" AND (statusCategory != Done) ORDER BY updated DESC"
        );
    }

    #[test]
    fn deserializes_realistic_search_fixture_into_output() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-basic.json")).unwrap();

        let output = output_from_search_response(response);

        assert_eq!(output.issues.len(), 3);
        assert_eq!(output.issues[0].key, "DEMO-101");
        assert_eq!(output.issues[0].status_name, "In Review");
        assert_eq!(output.issues[0].components, vec!["Web Platform"]);
    }

    #[test]
    fn deserializes_selected_optional_columns_when_present() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-columns.json")).unwrap();

        let output = output_from_search_response(response);

        assert_eq!(output.issues[0].issue_type_name.as_deref(), Some("Bug"));
        assert_eq!(
            output.issues[0].assignee_display_name.as_deref(),
            Some("Mina Li")
        );
        assert_eq!(output.issues[0].priority_name.as_deref(), Some("High"));
        assert_eq!(output.issues[1].assignee_display_name.as_deref(), None);
        assert_eq!(output.issues[1].priority_name.as_deref(), None);
    }

    #[test]
    fn deserialization_fails_when_required_summary_is_missing() {
        let error = serde_json::from_str::<SearchIssuesResponse<SearchIssueFields>>(&fixture(
            "search-missing-summary.json",
        ))
        .unwrap_err();

        assert!(error.to_string().contains("summary"));
    }

    #[test]
    fn deserialization_fails_when_status_shape_changes() {
        let error = serde_json::from_str::<SearchIssuesResponse<SearchIssueFields>>(&fixture(
            "search-invalid-status-shape.json",
        ))
        .unwrap_err();

        assert_eq!(error.classify(), serde_json::error::Category::Data);
    }

    #[test]
    fn render_human_includes_colorized_key_status_and_components_when_present() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-basic.json")).unwrap();
        let output = output_from_search_response(response);
        let mut rendered = Vec::new();

        render_human(&mut rendered, &output, &[], None).unwrap();

        let rendered = String::from_utf8(rendered).unwrap();
        assert!(rendered.contains("\u{1b}[1m\u{1b}[36mDEMO-101\u{1b}[0m [\u{1b}[33mIn Review\u{1b}[0m] Align application CSP with CDN configuration (\u{1b}[2mWeb Platform\u{1b}[0m)"));
        assert!(rendered.contains("\u{1b}[1m\u{1b}[36mDEMO-102\u{1b}[0m [\u{1b}[32mClosed\u{1b}[0m] Support iframe parent messaging (\u{1b}[2mWeb Platform\u{1b}[0m)"));
        assert!(rendered.ends_with("Next page token: sanitized-next-page-token\n"));
    }

    #[test]
    fn render_human_uses_selected_columns_and_colorizes_key_and_status() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-columns.json")).unwrap();
        let output = output_from_search_response(response);
        let mut rendered = Vec::new();

        render_human(
            &mut rendered,
            &output,
            &[
                SearchColumn::Key,
                SearchColumn::Type,
                SearchColumn::Status,
                SearchColumn::Assignee,
                SearchColumn::Priority,
                SearchColumn::Updated,
                SearchColumn::Summary,
            ],
            None,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            concat!(
                "\u{1b}[1m\u{1b}[36mDEMO-201\u{1b}[0m | Bug | \u{1b}[33mIn Progress\u{1b}[0m | Mina Li | High | 2026-06-22T14:45:00.000+0000 | Investigate webhook retries\n",
                "\u{1b}[1m\u{1b}[36mDEMO-202\u{1b}[0m | Task | \u{1b}[2mTo Do\u{1b}[0m | Unassigned | Unprioritized | 2026-06-21T09:15:00.000+0000 | Document fallback behavior\n"
            )
        );
    }

    #[test]
    fn render_human_omits_empty_components_suffix() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-no-components.json")).unwrap();
        let output = output_from_search_response(response);
        let mut rendered = Vec::new();

        render_human(&mut rendered, &output, &[], None).unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            "\u{1b}[1m\u{1b}[36mDEMO-104\u{1b}[0m [\u{1b}[32mClosed\u{1b}[0m] Populate missing environment values\n"
        );
    }

    #[test]
    fn render_human_colorizes_components_in_custom_columns() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-basic.json")).unwrap();
        let output = output_from_search_response(response);
        let mut rendered = Vec::new();

        render_human(
            &mut rendered,
            &output,
            &[
                SearchColumn::Key,
                SearchColumn::Components,
                SearchColumn::Summary,
            ],
            None,
        )
        .unwrap();

        assert!(String::from_utf8(rendered)
            .unwrap()
            .contains("\u{1b}[1m\u{1b}[36mDEMO-101\u{1b}[0m | \u{1b}[2mWeb Platform\u{1b}[0m | Align application CSP with CDN configuration"));
    }

    #[test]
    fn render_human_shows_empty_state_when_no_issues_match() {
        let output = SearchOutput {
            issues: Vec::new(),
            is_last: true,
            next_page_token: None,
        };
        let mut rendered = Vec::new();

        render_human(&mut rendered, &output, &[], None).unwrap();

        assert_eq!(String::from_utf8(rendered).unwrap(), "No issues found.\n");
    }

    #[test]
    fn render_human_shows_next_page_token_when_available() {
        let output = SearchOutput {
            issues: Vec::new(),
            is_last: false,
            next_page_token: Some("abc".to_string()),
        };
        let mut rendered = Vec::new();

        render_human(
            &mut rendered,
            &output,
            &[],
            Some("jeera search --next-page-token abc"),
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            "No issues found.\nNext page token: abc\nNext page command: jeera search --next-page-token abc\n"
        );
    }

    #[test]
    fn render_json_emits_stable_jeera_owned_schema() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-basic.json")).unwrap();
        let output = output_from_search_response(response);
        let mut rendered = Vec::new();

        render::render_json(&mut rendered, &output).unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            concat!(
                "{\n",
                "  \"issues\": [\n",
                "    {\n",
                "      \"key\": \"DEMO-101\",\n",
                "      \"summary\": \"Align application CSP with CDN configuration\",\n",
                "      \"status_name\": \"In Review\",\n",
                "      \"components\": [\n",
                "        \"Web Platform\"\n",
                "      ]\n",
                "    },\n",
                "    {\n",
                "      \"key\": \"DEMO-102\",\n",
                "      \"summary\": \"Support iframe parent messaging\",\n",
                "      \"status_name\": \"Closed\",\n",
                "      \"components\": [\n",
                "        \"Web Platform\"\n",
                "      ]\n",
                "    },\n",
                "    {\n",
                "      \"key\": \"DEMO-103\",\n",
                "      \"summary\": \"Adjust embedded content height\",\n",
                "      \"status_name\": \"Closed\",\n",
                "      \"components\": [\n",
                "        \"Web Platform\"\n",
                "      ]\n",
                "    }\n",
                "  ],\n",
                "  \"is_last\": false,\n",
                "  \"next_page_token\": \"sanitized-next-page-token\"\n",
                "}\n"
            )
        );
    }

    #[test]
    fn render_json_emits_additive_optional_fields_when_available() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-columns.json")).unwrap();
        let output = output_from_search_response(response);
        let mut rendered = Vec::new();

        render::render_json(&mut rendered, &output).unwrap();

        let rendered = String::from_utf8(rendered).unwrap();
        assert!(rendered.contains("\"issue_type_name\": \"Bug\""));
        assert!(rendered.contains("\"assignee_display_name\": \"Mina Li\""));
        assert!(rendered.contains("\"priority_name\": \"High\""));
        assert!(rendered.contains("\"updated\": \"2026-06-22T14:45:00.000+0000\""));
    }

    #[test]
    fn render_json_emits_empty_collection_for_no_matches() {
        let output = SearchOutput {
            issues: Vec::new(),
            is_last: true,
            next_page_token: None,
        };
        let mut rendered = Vec::new();

        render::render_json(&mut rendered, &output).unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            "{\n  \"issues\": [],\n  \"is_last\": true\n}\n"
        );
    }
}
