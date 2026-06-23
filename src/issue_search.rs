//! Jira issue search domain module.
//!
//! The command adapter enters through `prepare` and `execute_prepared`; tests and future
//! callers can use `prepare` when they only need the prepared Jira request.

use crate::cli::SearchArgs;
use crate::client::{
    JiraClient,
    types::{GetBoardConfigurationRequest, SearchIssuesRequest, SearchIssuesResponse},
};
use crate::error::AppError;
use crate::jql::{self, Clause, Order, Query, SortDirection, UserRef, Value};
use serde::{Deserialize, Serialize};
use std::io::Write;

const SEARCH_MIN_LIMIT: u32 = 1;
const SEARCH_MAX_LIMIT: u32 = 100;

/// Prepared search intent after validation, board resolution, JQL assembly, and field selection.
#[derive(Debug)]
pub struct PreparedIssueSearch {
    request: SearchIssuesRequest,
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

// Jira response shape requested by `search_fields`; keep these in lockstep.
#[derive(Debug, Deserialize)]
struct SearchIssueFields {
    summary: String,
    status: IssueStatus,
    #[serde(default)]
    components: Vec<IssueComponent>,
}

// Domain form of Jira board configuration, before it becomes JQL clauses.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BoardJqlFilter {
    filter_id: u64,
    sub_query: Option<String>,
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
    prepare_with_board_source(args, client.default_board_id(), |board_id| {
        board_filter(client, board_id)
    })
}

/// Human rendering is search-specific, while JSON rendering stays generic in `render`.
pub fn render_human(mut writer: impl Write, output: &SearchOutput) -> Result<(), AppError> {
    if output.issues.is_empty() {
        writeln!(writer, "No issues found.").map_err(|source| AppError::RenderOutput { source })?;
    } else {
        for issue in &output.issues {
            let components = issue.components.join(", ");

            writeln!(
                writer,
                "{} [{}] {}{}",
                issue.key,
                issue.status_name,
                issue.summary,
                if components.is_empty() {
                    String::new()
                } else {
                    format!(" ({components})")
                }
            )
            .map_err(|source| AppError::RenderOutput { source })?;
        }
    }

    if !output.is_last
        && let Some(next_page_token) = &output.next_page_token
    {
        writeln!(writer, "Next page token: {next_page_token}")
            .map_err(|source| AppError::RenderOutput { source })?;
    }

    Ok(())
}

impl PreparedIssueSearch {
    /// Exposes only the transport request; search preparation remains inside this module.
    pub fn request(&self) -> &SearchIssuesRequest {
        &self.request
    }

    pub fn jql(&self) -> &str {
        &self.request.jql
    }
}

// Internal seam for tests: production uses JiraClient, tests use a closure adapter.
fn prepare_with_board_source<F>(
    args: &SearchArgs,
    default_board_id: Option<u64>,
    load_board_filter: F,
) -> Result<PreparedIssueSearch, AppError>
where
    F: FnOnce(u64) -> Result<BoardJqlFilter, AppError>,
{
    validate_search_args(args)?;

    let configured_board_id = args.board.or(default_board_id);
    if configured_board_id.is_none() && !has_explicit_search_restriction(args) {
        return Err(AppError::InvalidSearch {
            reason: "provide at least one search restriction, such as --jql, --board, --project, --assignee, --component, --status, --label, --text, or configure default_board_id".to_string(),
        });
    }

    let board_filter = configured_board_id.map(load_board_filter).transpose()?;
    let jql = query_from_search_args(args, board_filter).to_jql();

    Ok(PreparedIssueSearch {
        request: SearchIssuesRequest {
            jql,
            next_page_token: args.next_page_token.clone(),
            max_results: Some(args.limit),
            fields: search_fields(),
            ..Default::default()
        },
    })
}

fn has_explicit_search_restriction(args: &SearchArgs) -> bool {
    args.jql
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
    validate_optional_value("jql", args.jql.as_deref())?;
    validate_optional_value("project", args.project.as_deref())?;
    validate_optional_value("assignee", args.assignee.as_deref())?;
    validate_optional_value("reporter", args.reporter.as_deref())?;
    validate_optional_value("status-category", args.status_category.as_deref())?;
    validate_optional_value("text", args.text.as_deref())?;
    validate_optional_value("next-page-token", args.next_page_token.as_deref())?;
    validate_repeated_values("status", &args.status)?;
    validate_repeated_values("type", &args.issue_type)?;
    validate_repeated_values("component", &args.component)?;
    validate_repeated_values("label", &args.label)?;
    validate_sort_field(&args.sort)?;
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

    if let Some(text) = &args.text {
        query.push(Clause::field_matches("text", text));
    }

    if args.open {
        query.push(Clause::raw("statusCategory != Done"));
    }

    let order_by = raw_order_by
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(
            || {
                Order::field(
                    args.sort.clone(),
                    if args.asc {
                        SortDirection::Asc
                    } else {
                        SortDirection::Desc
                    },
                )
            },
            Order::raw,
        );

    query.order_by(order_by);
    query
}

// Fields are part of the search output contract, not a caller option.
fn search_fields() -> Vec<String> {
    vec![
        "summary".to_string(),
        "status".to_string(),
        "components".to_string(),
    ]
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
        prepare_with_board_source(args, None, |_| unreachable!()).unwrap()
    }

    #[test]
    fn search_requires_an_explicit_or_configured_restriction() {
        let error = prepare_with_board_source(&SearchArgs::default(), None, |_| unreachable!())
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
        ] {
            let error = prepare_with_board_source(&args, None, |_| unreachable!()).unwrap_err();
            assert!(error.to_string().contains("cannot be empty"));
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
            let error = prepare_with_board_source(&args, None, |_| unreachable!()).unwrap_err();
            assert!(error.to_string().contains("cannot contain empty values"));
        }
    }

    #[test]
    fn search_rejects_invalid_sort_values() {
        for sort in ["", "   ", "updated desc", "updated,created"] {
            let error = prepare_with_board_source(
                &SearchArgs {
                    assignee: Some("me".to_string()),
                    sort: sort.to_string(),
                    ..Default::default()
                },
                None,
                |_| unreachable!(),
            )
            .unwrap_err();

            assert!(error.to_string().contains("--sort"));
        }
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
    fn explicit_desc_is_a_documented_no_op_because_descending_is_the_default() {
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
        let prepared = prepare_with_board_source(&args, Some(215), |board_id| {
            assert_eq!(board_id, 215);
            Ok(BoardJqlFilter {
                filter_id: 10492,
                sub_query: Some("fixVersion is EMPTY".to_string()),
            })
        })
        .unwrap();

        assert_eq!(
            prepared.request().jql,
            "filter = 10492 AND (fixVersion is EMPTY) AND component = \"QQMS\" ORDER BY updated DESC"
        );
    }

    #[test]
    fn final_jql_keeps_board_derived_clauses_when_combining_with_raw_jql() {
        let args = SearchArgs {
            jql: Some("project = GCCDEV ORDER BY Rank ASC".to_string()),
            component: vec!["QQMS".to_string()],
            ..Default::default()
        };
        let prepared = prepare_with_board_source(&args, Some(215), |board_id| {
            assert_eq!(board_id, 215);
            Ok(BoardJqlFilter {
                filter_id: 10492,
                sub_query: Some("fixVersion is EMPTY".to_string()),
            })
        })
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
    fn render_human_includes_components_when_present() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-basic.json")).unwrap();
        let output = output_from_search_response(response);
        let mut rendered = Vec::new();

        render_human(&mut rendered, &output).unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            concat!(
                "DEMO-101 [In Review] Align application CSP with CDN configuration (Web Platform)\n",
                "DEMO-102 [Closed] Support iframe parent messaging (Web Platform)\n",
                "DEMO-103 [Closed] Adjust embedded content height (Web Platform)\n",
                "Next page token: sanitized-next-page-token\n"
            )
        );
    }

    #[test]
    fn render_human_omits_empty_components_suffix() {
        let response: SearchIssuesResponse<SearchIssueFields> =
            serde_json::from_str(&fixture("search-no-components.json")).unwrap();
        let output = output_from_search_response(response);
        let mut rendered = Vec::new();

        render_human(&mut rendered, &output).unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            "DEMO-104 [Closed] Populate missing environment values\n"
        );
    }

    #[test]
    fn render_human_shows_empty_state_when_no_issues_match() {
        let output = SearchOutput {
            issues: Vec::new(),
            is_last: true,
            next_page_token: None,
        };
        let mut rendered = Vec::new();

        render_human(&mut rendered, &output).unwrap();

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

        render_human(&mut rendered, &output).unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            "No issues found.\nNext page token: abc\n"
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
