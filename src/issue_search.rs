//! Jira issue search domain module.
//!
//! The command adapter enters through `execute`; internal helpers prepare requests, merge
//! profiles, and render human output behind this module's interface.

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
use std::convert::TryFrom;
use std::io::Write;

const SEARCH_MIN_LIMIT: u32 = 1;
const SEARCH_MAX_LIMIT: u32 = 100;
const DEFAULT_SEARCH_LIMIT: u32 = 50;

/// Prepared search intent after validation, board resolution, JQL assembly, and field selection.
#[derive(Debug)]
struct PreparedIssueSearch {
    request: SearchIssuesRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchColumn {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchIntent {
    json: bool,
    debug_jql: bool,
    profile: Option<String>,
    query: Option<String>,
    jql: Option<String>,
    board: Option<BoardSelector>,
    project: Option<String>,
    assignee: Option<String>,
    unassigned: bool,
    reporter: Option<String>,
    status: Vec<String>,
    status_category: Option<String>,
    issue_type: Vec<String>,
    component: Vec<String>,
    label: Vec<String>,
    text: Option<String>,
    open: bool,
    limit: Option<u32>,
    next_page_token: Option<String>,
    human_columns: HumanColumns,
    sort: Option<String>,
    sort_direction: Option<SortDirection>,
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

/// Runs a complete Jira issue search behind the domain interface.
pub fn execute(client: &JiraClient, intent: SearchIntent) -> Result<SearchExecution, AppError> {
    let effective_intent = merge_search_profile(client, &intent)?;
    let prepared = prepare(client, &effective_intent)?;
    let output = execute_prepared(client, &prepared)?;
    let continuation = (!output.is_last)
        .then(|| output.next_page_token.clone())
        .flatten()
        .map(|next_page_token| SearchContinuation { next_page_token });

    Ok(SearchExecution {
        effective_intent,
        final_jql: prepared.jql().to_string(),
        output,
        continuation,
    })
}

/// Human rendering is search-specific, while JSON rendering stays generic in `render`.
pub fn render_human(mut writer: impl Write, execution: &SearchExecution) -> Result<(), AppError> {
    let effective_intent = execution.effective_intent();
    let next_page_command = execution
        .continuation()
        .map(|continuation| build_next_page_command(effective_intent, continuation));
    render_human_output(
        &mut writer,
        &execution.output,
        effective_intent.human_columns(),
        next_page_command.as_deref(),
    )
}

fn execute_prepared(
    client: &JiraClient,
    prepared: &PreparedIssueSearch,
) -> Result<SearchOutput, AppError> {
    let response = client
        .search_issues::<SearchIssueFields>(prepared.request())
        .map_err(|source| AppError::ExecuteSearch { source })?;

    Ok(output_from_search_response(response))
}

fn prepare(client: &JiraClient, intent: &SearchIntent) -> Result<PreparedIssueSearch, AppError> {
    prepare_with_board_source(
        intent,
        client.default_board_id(),
        |board_name| resolve_board_name(client, board_name),
        |board_id| board_filter(client, board_id),
    )
}

fn render_human_output(
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchOutputMode {
    json: bool,
}

impl SearchOutputMode {
    pub fn is_json(self) -> bool {
        self.json
    }
}

impl SearchIntent {
    fn human_columns(&self) -> &[SearchColumn] {
        match &self.human_columns {
            HumanColumns::Default => &[],
            HumanColumns::Custom(columns) => columns,
        }
    }

    fn to_search_args(&self) -> SearchArgs {
        SearchArgs {
            json: self.json,
            profile: self.profile.clone(),
            query: self.query.clone(),
            jql: self.jql.clone(),
            board: self.board.as_ref().map(BoardSelector::to_cli_value),
            project: self.project.clone(),
            assignee: self.assignee.clone(),
            unassigned: self.unassigned,
            reporter: self.reporter.clone(),
            status: self.status.clone(),
            status_category: self.status_category.clone(),
            issue_type: self.issue_type.clone(),
            component: self.component.clone(),
            label: self.label.clone(),
            text: self.text.clone(),
            open: self.open,
            limit: self.limit,
            next_page_token: self.next_page_token.clone(),
            columns: serialize_human_columns(&self.human_columns),
            debug_jql: self.debug_jql,
            sort: self.sort.clone(),
            asc: self.sort_direction == Some(SortDirection::Asc),
            desc: self.sort_direction == Some(SortDirection::Desc),
        }
    }
}

impl SearchContinuation {
    pub fn next_page_token(&self) -> &str {
        &self.next_page_token
    }
}

impl PreparedIssueSearch {
    fn request(&self) -> &SearchIssuesRequest {
        &self.request
    }

    fn jql(&self) -> &str {
        &self.request.jql
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

impl TryFrom<&SearchArgs> for SearchIntent {
    type Error = AppError;

    fn try_from(args: &SearchArgs) -> Result<Self, Self::Error> {
        Ok(Self {
            json: args.json,
            debug_jql: args.debug_jql,
            profile: args.profile.clone(),
            query: args.query.clone(),
            jql: args.jql.clone(),
            board: args
                .board
                .as_deref()
                .map(str::trim)
                .map(parse_board_selector)
                .transpose()?,
            project: args.project.clone(),
            assignee: args.assignee.clone(),
            unassigned: args.unassigned,
            reporter: args.reporter.clone(),
            status: args.status.clone(),
            status_category: args.status_category.clone(),
            issue_type: args.issue_type.clone(),
            component: args.component.clone(),
            label: args.label.clone(),
            text: args.text.clone(),
            open: args.open,
            limit: args.limit,
            next_page_token: args.next_page_token.clone(),
            human_columns: parse_human_columns(args.columns.as_deref())?,
            sort: args.sort.clone(),
            sort_direction: if args.asc {
                Some(SortDirection::Asc)
            } else if args.desc {
                Some(SortDirection::Desc)
            } else {
                None
            },
        })
    }
}

impl BoardSelector {
    fn to_cli_value(&self) -> String {
        match self {
            Self::Id(board_id) => board_id.to_string(),
            Self::Name(board_name) => board_name.clone(),
        }
    }
}

fn merge_search_profile(
    client: &JiraClient,
    intent: &SearchIntent,
) -> Result<SearchIntent, AppError> {
    let args = intent.to_search_args();
    let Some(profile_name) = args.profile.as_deref() else {
        return Ok(intent.clone());
    };

    let profile = client
        .search_profile(profile_name)
        .ok_or_else(|| AppError::InvalidSearch {
            reason: format!("unknown search profile {profile_name:?}"),
        })?;

    let (assignee, unassigned) = if args.unassigned {
        (None, true)
    } else if let Some(assignee) = &args.assignee {
        (Some(assignee.clone()), false)
    } else {
        (profile.assignee.clone(), profile.unassigned)
    };

    let (asc, desc) = if args.asc {
        (true, false)
    } else if args.desc {
        (false, true)
    } else {
        (profile.asc, profile.desc)
    };

    SearchIntent::try_from(&SearchArgs {
        json: args.json,
        profile: None,
        query: args.query.clone(),
        jql: args.jql.clone().or_else(|| profile.jql.clone()),
        board: args.board.clone().or_else(|| profile.board.clone()),
        project: args.project.clone().or_else(|| profile.project.clone()),
        assignee,
        unassigned,
        reporter: args.reporter.clone().or_else(|| profile.reporter.clone()),
        status: merged_vec(&profile.status, &args.status),
        status_category: args
            .status_category
            .clone()
            .or_else(|| profile.status_category.clone()),
        issue_type: merged_vec(&profile.issue_type, &args.issue_type),
        component: merged_vec(&profile.component, &args.component),
        label: merged_vec(&profile.label, &args.label),
        text: args.text.clone().or_else(|| profile.text.clone()),
        open: args.open || profile.open,
        limit: args.limit.or(profile.limit),
        next_page_token: args.next_page_token.clone(),
        columns: args.columns.clone(),
        debug_jql: args.debug_jql,
        sort: args.sort.clone().or_else(|| profile.sort.clone()),
        asc,
        desc,
    })
}

fn merged_vec(profile_values: &[String], cli_values: &[String]) -> Vec<String> {
    profile_values
        .iter()
        .chain(cli_values.iter())
        .cloned()
        .collect()
}

fn serialize_human_columns(human_columns: &HumanColumns) -> Option<String> {
    match human_columns {
        HumanColumns::Default => None,
        HumanColumns::Custom(columns) => Some(
            columns
                .iter()
                .map(|column| match column {
                    SearchColumn::Key => "key",
                    SearchColumn::Status => "status",
                    SearchColumn::Summary => "summary",
                    SearchColumn::Components => "components",
                    SearchColumn::Type => "type",
                    SearchColumn::Assignee => "assignee",
                    SearchColumn::Priority => "priority",
                    SearchColumn::Updated => "updated",
                })
                .collect::<Vec<_>>()
                .join(","),
        ),
    }
}

fn build_next_page_command(intent: &SearchIntent, continuation: &SearchContinuation) -> String {
    let mut parts = vec!["jeera".to_string(), "search".to_string()];

    if intent.json {
        parts.push("--json".to_string());
    }
    if let Some(jql) = &intent.jql {
        parts.push("--jql".to_string());
        parts.push(shell_quote(jql));
    }
    if let Some(board) = &intent.board {
        parts.push("--board".to_string());
        parts.push(shell_quote(&board.to_cli_value()));
    }
    if let Some(project) = &intent.project {
        parts.push("--project".to_string());
        parts.push(shell_quote(project));
    }
    if let Some(assignee) = &intent.assignee {
        parts.push("--assignee".to_string());
        parts.push(shell_quote(assignee));
    }
    if intent.unassigned {
        parts.push("--unassigned".to_string());
    }
    if let Some(reporter) = &intent.reporter {
        parts.push("--reporter".to_string());
        parts.push(shell_quote(reporter));
    }
    for status in &intent.status {
        parts.push("--status".to_string());
        parts.push(shell_quote(status));
    }
    if let Some(status_category) = &intent.status_category {
        parts.push("--status-category".to_string());
        parts.push(shell_quote(status_category));
    }
    for issue_type in &intent.issue_type {
        parts.push("--type".to_string());
        parts.push(shell_quote(issue_type));
    }
    for component in &intent.component {
        parts.push("--component".to_string());
        parts.push(shell_quote(component));
    }
    for label in &intent.label {
        parts.push("--label".to_string());
        parts.push(shell_quote(label));
    }
    if let Some(text) = &intent.text {
        parts.push("--text".to_string());
        parts.push(shell_quote(text));
    }
    if intent.open {
        parts.push("--open".to_string());
    }
    parts.push("--limit".to_string());
    parts.push(intent.limit.unwrap_or(DEFAULT_SEARCH_LIMIT).to_string());
    if let Some(columns) = serialize_human_columns(&intent.human_columns) {
        parts.push("--columns".to_string());
        parts.push(shell_quote(&columns));
    }
    if let Some(sort) = &intent.sort {
        parts.push("--sort".to_string());
        parts.push(shell_quote(sort));
    }
    if intent.sort_direction == Some(SortDirection::Asc) {
        parts.push("--asc".to_string());
    }
    if intent.sort_direction == Some(SortDirection::Desc) {
        parts.push("--desc".to_string());
    }
    parts.push("--next-page-token".to_string());
    parts.push(shell_quote(continuation.next_page_token()));
    if let Some(query) = &intent.query {
        parts.push(shell_quote(query));
    }

    parts.join(" ")
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty() && value.bytes().all(|byte| {
        matches!(
            byte,
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b':' | b'@' | b'='
        )
    }) {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
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
    intent: &SearchIntent,
    default_board_id: Option<u64>,
    resolve_board_name: R,
    load_board_filter: F,
) -> Result<PreparedIssueSearch, AppError>
where
    R: FnMut(&str) -> Result<u64, AppError>,
    F: FnOnce(u64) -> Result<BoardJqlFilter, AppError>,
{
    validate_search_intent(intent)?;

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
            max_results: Some(intent.limit.unwrap_or(DEFAULT_SEARCH_LIMIT)),
            fields: search_fields(intent.json, &intent.human_columns),
            ..Default::default()
        },
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

fn has_explicit_search_restriction(intent: &SearchIntent) -> bool {
    intent
        .query
        .as_deref()
        .is_some_and(|query| !query.trim().is_empty())
        || intent
            .jql
            .as_deref()
            .is_some_and(|jql| !jql.trim().is_empty())
        || intent.project.is_some()
        || intent.assignee.is_some()
        || intent.unassigned
        || intent.reporter.is_some()
        || !intent.status.is_empty()
        || intent.status_category.is_some()
        || !intent.issue_type.is_empty()
        || !intent.component.is_empty()
        || !intent.label.is_empty()
        || intent.text.is_some()
        || intent.open
}

fn validate_search_intent(intent: &SearchIntent) -> Result<(), AppError> {
    validate_limit(intent.limit.unwrap_or(DEFAULT_SEARCH_LIMIT))?;
    validate_optional_value("query", intent.query.as_deref())?;
    validate_optional_value("jql", intent.jql.as_deref())?;
    validate_optional_value(
        "board",
        intent.board.as_ref().map(|board| match board {
            BoardSelector::Id(_) => "id",
            BoardSelector::Name(board_name) => board_name.as_str(),
        }),
    )?;
    validate_optional_value("project", intent.project.as_deref())?;
    validate_optional_value("assignee", intent.assignee.as_deref())?;
    validate_optional_value("reporter", intent.reporter.as_deref())?;
    validate_optional_value("status-category", intent.status_category.as_deref())?;
    validate_optional_value("text", intent.text.as_deref())?;
    validate_optional_value("next-page-token", intent.next_page_token.as_deref())?;
    validate_repeated_values("status", &intent.status)?;
    validate_repeated_values("type", &intent.issue_type)?;
    validate_repeated_values("component", &intent.component)?;
    validate_repeated_values("label", &intent.label)?;
    if let Some(sort) = intent.sort.as_deref() {
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
    board: Option<&BoardSelector>,
    default_board_id: Option<u64>,
    mut resolve_board_name: R,
) -> Result<Option<u64>, AppError>
where
    R: FnMut(&str) -> Result<u64, AppError>,
{
    match board {
        Some(BoardSelector::Id(board_id)) => Ok(Some(*board_id)),
        Some(BoardSelector::Name(board_name)) => resolve_board_name(board_name).map(Some),
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
fn query_from_search_intent(intent: &SearchIntent, board_filter: Option<BoardJqlFilter>) -> Query {
    let board_scoped = board_filter.is_some();
    let (raw_clause, raw_order_by) = intent
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

    if let Some(project) = &intent.project {
        query.push(Clause::field_equals("project", Value::text(project)));
    }

    if intent.unassigned {
        query.push(Clause::is_empty("assignee"));
    } else if let Some(assignee) = &intent.assignee {
        query.push(Clause::field_equals(
            "assignee",
            UserRef::parse(assignee).to_value(),
        ));
    }

    if let Some(reporter) = &intent.reporter {
        query.push(Clause::field_equals(
            "reporter",
            UserRef::parse(reporter).to_value(),
        ));
    }

    if !intent.status.is_empty() {
        query.push(Clause::field_in("status", intent.status.clone()));
    }

    if let Some(status_category) = &intent.status_category {
        query.push(Clause::field_equals(
            "statusCategory",
            Value::text(status_category),
        ));
    }

    if !intent.issue_type.is_empty() {
        query.push(Clause::field_in("issuetype", intent.issue_type.clone()));
    }

    if !intent.component.is_empty() {
        query.push(Clause::field_in("component", intent.component.clone()));
    }

    if !intent.label.is_empty() {
        query.push(Clause::field_in("labels", intent.label.clone()));
    }

    if let Some(query_text) = &intent.query {
        query.push(Clause::field_matches("text", query_text));
    }

    if let Some(text) = &intent.text {
        query.push(Clause::field_matches("text", text));
    }

    if intent.open {
        query.push(Clause::raw("statusCategory != Done"));
    }

    let order_by = raw_order_by
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(|| default_order(intent, board_scoped), Order::raw);

    query.order_by(order_by);
    query
}

fn default_order(intent: &SearchIntent, board_scoped: bool) -> Order {
    let field = intent
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

    let direction = if intent.sort_direction == Some(SortDirection::Asc) {
        SortDirection::Asc
    } else if intent.sort_direction == Some(SortDirection::Desc) {
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
    use crate::client::{JiraAuth, JiraClient, JiraClientConfig};
    use crate::config::SearchProfileSettings;
    use crate::render;
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::thread;
    use std::time::Duration;
    use url::Url;

    fn fixture(path: &str) -> String {
        fs::read_to_string(Path::new("tests/fixtures/jira").join(path)).unwrap()
    }

    fn spawn_server(status_line: &str, body: &str) -> (Url, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let (tx, rx) = std::sync::mpsc::channel();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();

            let mut buffer = [0_u8; 8192];
            let bytes_read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..bytes_read]).into_owned();
            tx.send(request).unwrap();
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });

        (Url::parse(&format!("http://{addr}/")).unwrap(), rx)
    }

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
        super::prepare_with_board_source(
            &SearchIntent::try_from(args)?,
            default_board_id,
            resolve_board_name,
            load_board_filter,
        )
    }

    fn prepare_without_board(args: &SearchArgs) -> PreparedIssueSearch {
        prepare_with_board_source(args, None, |_| unreachable!(), |_| unreachable!()).unwrap()
    }

    fn render_human(
        writer: impl Write,
        output: &SearchOutput,
        columns: &[SearchColumn],
        next_page_command: Option<&str>,
    ) -> Result<(), AppError> {
        super::render_human_output(writer, output, columns, next_page_command)
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
                limit: Some(0),
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
                limit: Some(101),
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
                board: Some("SAMPLE Kanban Board".to_string()),
                component: vec!["QQMS".to_string()],
                ..Default::default()
            },
            None,
            |board_name| {
                assert_eq!(board_name, "SAMPLE Kanban Board");
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
            name: "SAMPLE Kanban Board".to_string(),
            board_type: "kanban".to_string(),
            location: None,
        }];

        assert_eq!(
            find_board_id_by_name(&boards, "sample kanban board").unwrap(),
            215
        );
    }

    #[test]
    fn unknown_board_name_is_reported_clearly() {
        let boards = vec![BoardResponse {
            id: 215,
            name: "SAMPLE Kanban Board".to_string(),
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
            jql: Some("project = SAMPLE ORDER BY Rank ASC".to_string()),
            ..Default::default()
        });

        assert_eq!(
            prepared.request().jql,
            "(project = SAMPLE) AND text ~ \"reporting\" ORDER BY Rank ASC"
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
            limit: Some(25),
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
            jql: Some("project = SAMPLE ORDER BY Rank ASC".to_string()),
            component: vec!["QQMS".to_string()],
            ..Default::default()
        };
        let prepared = prepare_without_board(&args);

        assert_eq!(
            prepared.request().jql,
            "(project = SAMPLE) AND component = \"QQMS\" ORDER BY Rank ASC"
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
            jql: Some("project = SAMPLE ORDER BY Rank ASC".to_string()),
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
            "(project = SAMPLE) AND filter = 10492 AND (fixVersion is EMPTY) AND component = \"QQMS\" ORDER BY Rank ASC"
        );
    }

    #[test]
    fn structured_filters_are_combined_and_values_are_escaped() {
        let args = SearchArgs {
            project: Some("SAMPLE".to_string()),
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
            "project = \"SAMPLE\" AND assignee = currentUser() AND status in (\"In Progress\", \"Ready \\\"Soon\\\"\") AND component = \"QQMS\" AND text ~ \"reporting\" AND (statusCategory != Done) ORDER BY updated DESC"
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
    fn execute_exposes_effective_intent_and_expands_continuation_from_it() {
        let body = r#"{"isLast":false,"nextPageToken":"next token","issues":[]}"#;
        let (base_url, rx) = spawn_server("200 OK", body);
        let mut searches = BTreeMap::new();
        searches.insert(
            "qqms".to_string(),
            SearchProfileSettings {
                project: Some("SAMPLE".to_string()),
                component: vec!["QQMS".to_string()],
                limit: Some(25),
                ..Default::default()
            },
        );
        let client = JiraClient::new(JiraClientConfig {
            base_url,
            auth: JiraAuth::Basic {
                email: "user@example.com".to_string(),
                api_token: "token".to_string(),
            },
            timeout: Duration::from_secs(5),
            default_board_id: None,
            searches,
        });
        let intent = SearchIntent::try_from(&SearchArgs {
            profile: Some("qqms".to_string()),
            ..Default::default()
        })
        .unwrap();

        let execution = execute(&client, intent).unwrap();

        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            execution.effective_intent().project.as_deref(),
            Some("SAMPLE")
        );
        assert_eq!(execution.effective_intent().component, vec!["QQMS"]);
        assert_eq!(execution.effective_intent().limit, Some(25));
        assert_eq!(
            execution.continuation().unwrap().next_page_token(),
            "next token"
        );

        let mut rendered = Vec::new();
        super::render_human(&mut rendered, &execution).unwrap();
        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            "No issues found.\nNext page token: next token\nNext page command: jeera search --project SAMPLE --component QQMS --limit 25 --next-page-token 'next token'\n"
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
