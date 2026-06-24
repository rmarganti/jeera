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
