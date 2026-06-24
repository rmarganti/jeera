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
        let error = prepare_with_board_source(&args, None, |_| unreachable!(), |_| unreachable!())
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
        let error = prepare_with_board_source(&args, None, |_| unreachable!(), |_| unreachable!())
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
