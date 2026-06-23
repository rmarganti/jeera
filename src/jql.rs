//! Generic JQL rendering module.
//!
//! Domain modules decide which clauses they need; this module only knows how to render them.

/// A composable JQL query: ordered clauses plus an optional trailing `ORDER BY`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Query {
    clauses: Vec<Clause>,
    order_by: Option<Order>,
}

/// Generic JQL clause vocabulary shared by domain modules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clause {
    Raw(String),
    FieldEquals { field: String, value: Value },
    FieldIn { field: String, values: Vec<String> },
    FieldMatches { field: String, value: String },
    IsEmpty { field: String },
}

/// JQL values that need distinct rendering rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Function(String),
    Number(u64),
}

/// Sort expression; `Raw` preserves user-supplied order clauses without re-parsing Jira syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Order {
    Field {
        field: String,
        direction: SortDirection,
    },
    Raw(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

/// Shared user shorthand used by Jira filters such as assignee and reporter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserRef {
    CurrentUser,
    Named(String),
}

impl Query {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, clause: Clause) {
        self.clauses.push(clause);
    }

    pub fn order_by(&mut self, order: Order) {
        self.order_by = Some(order);
    }

    /// Renders the complete JQL string used at the Jira transport seam.
    pub fn to_jql(&self) -> String {
        let clauses = self.clauses.iter().map(Clause::to_jql).collect::<Vec<_>>();

        let mut jql = clauses.join(" AND ");

        if let Some(order_by) = &self.order_by {
            if !jql.is_empty() {
                jql.push(' ');
            }
            jql.push_str(&order_by.to_jql());
        }

        jql
    }
}

impl Clause {
    /// Preserves caller-owned JQL while still composing it with structured clauses.
    pub fn raw(value: impl Into<String>) -> Self {
        Self::Raw(value.into())
    }

    pub fn field_equals(field: impl Into<String>, value: Value) -> Self {
        Self::FieldEquals {
            field: field.into(),
            value,
        }
    }

    pub fn field_in(field: impl Into<String>, values: Vec<String>) -> Self {
        Self::FieldIn {
            field: field.into(),
            values,
        }
    }

    pub fn field_matches(field: impl Into<String>, value: impl Into<String>) -> Self {
        Self::FieldMatches {
            field: field.into(),
            value: value.into(),
        }
    }

    pub fn is_empty(field: impl Into<String>) -> Self {
        Self::IsEmpty {
            field: field.into(),
        }
    }

    fn to_jql(&self) -> String {
        match self {
            Self::Raw(value) => format!("({value})"),
            Self::FieldEquals { field, value } => format!("{field} = {}", value.to_jql()),
            Self::FieldIn { field, values } => field_values(field, values),
            Self::FieldMatches { field, value } => format!("{field} ~ {}", quote(value)),
            Self::IsEmpty { field } => format!("{field} is EMPTY"),
        }
    }
}

impl Value {
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    pub fn function(value: impl Into<String>) -> Self {
        Self::Function(value.into())
    }

    pub fn number(value: u64) -> Self {
        Self::Number(value)
    }

    fn to_jql(&self) -> String {
        match self {
            Self::Text(value) => quote(value),
            Self::Function(value) => value.clone(),
            Self::Number(value) => value.to_string(),
        }
    }
}

impl Order {
    pub fn field(field: impl Into<String>, direction: SortDirection) -> Self {
        Self::Field {
            field: field.into(),
            direction,
        }
    }

    pub fn raw(value: impl Into<String>) -> Self {
        Self::Raw(value.into())
    }

    fn to_jql(&self) -> String {
        match self {
            Self::Field { field, direction } => {
                format!("ORDER BY {} {}", field, direction.to_jql())
            }
            Self::Raw(value) => format!("ORDER BY {value}"),
        }
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

impl UserRef {
    pub fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("me") || value.eq_ignore_ascii_case("currentUser()") {
            Self::CurrentUser
        } else {
            Self::Named(value.to_string())
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            Self::CurrentUser => Value::function("currentUser()"),
            Self::Named(value) => Value::text(value),
        }
    }
}

/// Splits user JQL so domain modules can combine raw filters with their own ordering rules.
pub fn split_order_by(jql: &str) -> (Option<&str>, Option<&str>) {
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

    #[test]
    fn empty_query_can_still_sort() {
        let mut query = Query::new();
        query.order_by(Order::field("updated", SortDirection::Desc));

        assert_eq!(query.to_jql(), "ORDER BY updated DESC");
    }

    #[test]
    fn raw_jql_can_be_combined_with_structured_clauses() {
        let mut query = Query::new();
        query.push(Clause::raw("project = SAMPLE"));
        query.push(Clause::field_in("component", vec!["QQMS".to_string()]));
        query.order_by(Order::field("Rank", SortDirection::Asc));

        assert_eq!(
            query.to_jql(),
            "(project = SAMPLE) AND component = \"QQMS\" ORDER BY Rank ASC"
        );
    }

    #[test]
    fn structured_clauses_are_combined_and_values_are_escaped() {
        let mut query = Query::new();
        query.push(Clause::field_equals("project", Value::text("SAMPLE")));
        query.push(Clause::field_equals(
            "assignee",
            UserRef::parse("me").to_value(),
        ));
        query.push(Clause::field_in(
            "status",
            vec!["In Progress".to_string(), "Ready \"Soon\"".to_string()],
        ));
        query.push(Clause::field_in("component", vec!["QQMS".to_string()]));
        query.push(Clause::field_matches("text", "reporting"));
        query.push(Clause::raw("statusCategory != Done"));
        query.order_by(Order::field("updated", SortDirection::Desc));

        assert_eq!(
            query.to_jql(),
            "project = \"SAMPLE\" AND assignee = currentUser() AND status in (\"In Progress\", \"Ready \\\"Soon\\\"\") AND component = \"QQMS\" AND text ~ \"reporting\" AND (statusCategory != Done) ORDER BY updated DESC"
        );
    }

    #[test]
    fn splits_trailing_order_by_case_insensitively() {
        assert_eq!(
            split_order_by("project = DEMO ORDER BY Rank ASC"),
            (Some("project = DEMO"), Some("Rank ASC"))
        );
    }
}
