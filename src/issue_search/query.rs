use super::board::BoardJqlFilter;
use super::intent::{HumanColumns, SearchColumn, SearchIntent};
use crate::jql::{self, Clause, Order, Query, SortDirection, UserRef, Value};
use std::borrow::Cow;

// Translates CLI-shaped search intent into generic JQL clauses.
pub(super) fn query_from_search_intent(
    intent: &SearchIntent,
    board_filter: Option<BoardJqlFilter>,
) -> Query {
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

pub(super) fn search_fields(json: bool, human_columns: &HumanColumns) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use crate::cli::SearchArgs;
    use crate::issue_search::tests_support::{
        board_filter, prepare_with_board_source_for_args, prepare_without_board,
    };

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
        let prepared = prepare_with_board_source_for_args(
            &SearchArgs {
                query: Some("reporting".to_string()),
                ..Default::default()
            },
            Some(215),
            |_| unreachable!(),
            |board_id| {
                assert_eq!(board_id, 215);
                Ok(board_filter(10492, "fixVersion is EMPTY"))
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
        let prepared = prepare_with_board_source_for_args(
            &SearchArgs {
                component: vec!["QQMS".to_string()],
                ..Default::default()
            },
            Some(215),
            |_| unreachable!(),
            |board_id| {
                assert_eq!(board_id, 215);
                Ok(board_filter(10492, "fixVersion is EMPTY"))
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
        let prepared = prepare_with_board_source_for_args(
            &SearchArgs {
                component: vec!["QQMS".to_string()],
                sort: Some("updated".to_string()),
                ..Default::default()
            },
            Some(215),
            |_| unreachable!(),
            |board_id| {
                assert_eq!(board_id, 215);
                Ok(board_filter(10492, "fixVersion is EMPTY"))
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
        let prepared = prepare_with_board_source_for_args(
            &SearchArgs {
                component: vec!["QQMS".to_string()],
                desc: true,
                ..Default::default()
            },
            Some(215),
            |_| unreachable!(),
            |board_id| {
                assert_eq!(board_id, 215);
                Ok(board_filter(10492, "fixVersion is EMPTY"))
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
        let prepared = prepare_with_board_source_for_args(
            &args,
            Some(215),
            |_| unreachable!(),
            |board_id| {
                assert_eq!(board_id, 215);
                Ok(board_filter(10492, "fixVersion is EMPTY"))
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
        let prepared = prepare_with_board_source_for_args(
            &args,
            Some(215),
            |_| unreachable!(),
            |board_id| {
                assert_eq!(board_id, 215);
                Ok(board_filter(10492, "fixVersion is EMPTY"))
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
}
