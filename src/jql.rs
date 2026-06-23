use crate::cli::SearchArgs;
use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueQuery {
    filters: Vec<JqlFilter>,
    order_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardJqlFilter {
    pub filter_id: u64,
    pub sub_query: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JqlFilter {
    Raw(String),
    BoardFilter(u64),
    BoardSubQuery(String),
    Project(String),
    Assignee(UserRef),
    Reporter(UserRef),
    Status(Vec<String>),
    StatusCategory(String),
    IssueType(Vec<String>),
    Component(Vec<String>),
    Label(Vec<String>),
    Text(String),
    Open,
    Unassigned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserRef {
    CurrentUser,
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JqlOrder {
    field: String,
    direction: SortDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

impl IssueQuery {
    pub fn from_search_args(
        args: &SearchArgs,
        board_filter: Option<BoardJqlFilter>,
    ) -> Result<Self, AppError> {
        let (raw_clause, raw_order_by) =
            args.jql.as_deref().map(split_order_by).unwrap_or_default();
        let mut filters = Vec::new();

        if let Some(raw_clause) = raw_clause
            && !raw_clause.trim().is_empty()
        {
            filters.push(JqlFilter::Raw(raw_clause.trim().to_string()));
        }

        if let Some(board_filter) = board_filter {
            filters.push(JqlFilter::BoardFilter(board_filter.filter_id));
            if let Some(sub_query) = board_filter.sub_query
                && !sub_query.trim().is_empty()
            {
                filters.push(JqlFilter::BoardSubQuery(sub_query));
            }
        }

        if let Some(project) = &args.project {
            filters.push(JqlFilter::Project(project.clone()));
        }

        if args.unassigned {
            filters.push(JqlFilter::Unassigned);
        } else if let Some(assignee) = &args.assignee {
            filters.push(JqlFilter::Assignee(parse_user_ref(assignee)));
        }

        if let Some(reporter) = &args.reporter {
            filters.push(JqlFilter::Reporter(parse_user_ref(reporter)));
        }

        if !args.status.is_empty() {
            filters.push(JqlFilter::Status(args.status.clone()));
        }

        if let Some(status_category) = &args.status_category {
            filters.push(JqlFilter::StatusCategory(status_category.clone()));
        }

        if !args.issue_type.is_empty() {
            filters.push(JqlFilter::IssueType(args.issue_type.clone()));
        }

        if !args.component.is_empty() {
            filters.push(JqlFilter::Component(args.component.clone()));
        }

        if !args.label.is_empty() {
            filters.push(JqlFilter::Label(args.label.clone()));
        }

        if let Some(text) = &args.text {
            filters.push(JqlFilter::Text(text.clone()));
        }

        if args.open {
            filters.push(JqlFilter::Open);
        }

        let order_by = raw_order_by
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map_or_else(
                || {
                    Some(
                        JqlOrder {
                            field: args.sort.clone(),
                            direction: if args.asc {
                                SortDirection::Asc
                            } else {
                                SortDirection::Desc
                            },
                        }
                        .to_jql(),
                    )
                },
                |order_by| Some(format!("ORDER BY {order_by}")),
            );

        Ok(Self { filters, order_by })
    }

    pub fn to_jql(&self) -> String {
        let clauses = self
            .filters
            .iter()
            .map(JqlFilter::to_jql)
            .collect::<Vec<_>>();

        let mut jql = clauses.join(" AND ");

        if let Some(order_by) = &self.order_by {
            if !jql.is_empty() {
                jql.push(' ');
            }
            jql.push_str(order_by);
        }

        jql
    }
}

impl JqlFilter {
    fn to_jql(&self) -> String {
        match self {
            Self::Raw(value) => format!("({value})"),
            Self::BoardFilter(filter_id) => format!("filter = {filter_id}"),
            Self::BoardSubQuery(query) => format!("({query})"),
            Self::Project(value) => format!("project = {}", quote(value)),
            Self::Assignee(user) => format!("assignee = {}", user.to_jql()),
            Self::Reporter(user) => format!("reporter = {}", user.to_jql()),
            Self::Status(values) => field_values("status", values),
            Self::StatusCategory(value) => format!("statusCategory = {}", quote(value)),
            Self::IssueType(values) => field_values("issuetype", values),
            Self::Component(values) => field_values("component", values),
            Self::Label(values) => field_values("labels", values),
            Self::Text(value) => format!("text ~ {}", quote(value)),
            Self::Open => "statusCategory != Done".to_string(),
            Self::Unassigned => "assignee is EMPTY".to_string(),
        }
    }
}

impl UserRef {
    fn to_jql(&self) -> String {
        match self {
            Self::CurrentUser => "currentUser()".to_string(),
            Self::Named(value) => quote(value),
        }
    }
}

impl JqlOrder {
    fn to_jql(&self) -> String {
        format!("ORDER BY {} {}", self.field, self.direction.to_jql())
    }
}

impl SortDirection {
    fn to_jql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

fn split_order_by(jql: &str) -> (Option<&str>, Option<&str>) {
    let lower = jql.to_ascii_lowercase();
    if let Some(index) = lower.rfind(" order by ") {
        (
            Some(&jql[..index]),
            Some(&jql[index + " order by ".len()..]),
        )
    } else {
        (Some(jql), None)
    }
}

fn parse_user_ref(value: &str) -> UserRef {
    if value.eq_ignore_ascii_case("me") || value.eq_ignore_ascii_case("currentUser()") {
        UserRef::CurrentUser
    } else {
        UserRef::Named(value.to_string())
    }
}

fn field_values(field: &str, values: &[String]) -> String {
    if values.len() == 1 {
        format!("{field} = {}", quote(&values[0]))
    } else {
        format!(
            "{field} in ({})",
            values
                .iter()
                .map(|value| quote(value))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn quote(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::SearchArgs;

    #[test]
    fn default_query_has_no_implicit_filters() {
        let query = IssueQuery::from_search_args(&SearchArgs::default(), None).unwrap();

        assert_eq!(query.to_jql(), "ORDER BY updated DESC");
    }

    #[test]
    fn raw_jql_can_be_combined_with_explicit_filters() {
        let args = SearchArgs {
            jql: Some("project = GCCDEV ORDER BY Rank ASC".to_string()),
            component: vec!["QQMS".to_string()],
            ..Default::default()
        };

        let query = IssueQuery::from_search_args(&args, None).unwrap();

        assert_eq!(
            query.to_jql(),
            "(project = GCCDEV) AND component = \"QQMS\" ORDER BY Rank ASC"
        );
    }

    #[test]
    fn board_filter_is_just_another_jql_clause() {
        let args = SearchArgs {
            component: vec!["QQMS".to_string()],
            ..Default::default()
        };

        let query = IssueQuery::from_search_args(
            &args,
            Some(BoardJqlFilter {
                filter_id: 10492,
                sub_query: Some("fixVersion is EMPTY".to_string()),
            }),
        )
        .unwrap();

        assert_eq!(
            query.to_jql(),
            "filter = 10492 AND (fixVersion is EMPTY) AND component = \"QQMS\" ORDER BY updated DESC"
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

        let query = IssueQuery::from_search_args(&args, None).unwrap();

        assert_eq!(
            query.to_jql(),
            "project = \"GCCDEV\" AND assignee = currentUser() AND status in (\"In Progress\", \"Ready \\\"Soon\\\"\") AND component = \"QQMS\" AND text ~ \"reporting\" AND statusCategory != Done ORDER BY updated DESC"
        );
    }
}
