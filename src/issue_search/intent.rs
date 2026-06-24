use super::board::{BoardSelector, parse_board_selector};
use super::output::SearchIssueOutput;
use crate::cli::SearchArgs;
use crate::error::AppError;
use crate::jql::SortDirection;
use std::convert::TryFrom;

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
pub(super) enum HumanColumns {
    Default,
    Custom(Vec<SearchColumn>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchIntent {
    pub(super) json: bool,
    pub(super) debug_jql: bool,
    pub(super) profile: Option<String>,
    pub(super) query: Option<String>,
    pub(super) jql: Option<String>,
    pub(super) board: Option<BoardSelector>,
    pub(super) project: Option<String>,
    pub(super) assignee: Option<String>,
    pub(super) unassigned: bool,
    pub(super) reporter: Option<String>,
    pub(super) status: Vec<String>,
    pub(super) status_category: Option<String>,
    pub(super) issue_type: Vec<String>,
    pub(super) component: Vec<String>,
    pub(super) label: Vec<String>,
    pub(super) text: Option<String>,
    pub(super) open: bool,
    pub(super) limit: Option<u32>,
    pub(super) next_page_token: Option<String>,
    pub(super) human_columns: HumanColumns,
    pub(super) sort: Option<String>,
    pub(super) sort_direction: Option<SortDirection>,
}

impl SearchIntent {
    pub(super) fn human_columns(&self) -> &[SearchColumn] {
        match &self.human_columns {
            HumanColumns::Default => &[],
            HumanColumns::Custom(columns) => columns,
        }
    }

    pub(super) fn to_search_args(&self) -> SearchArgs {
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

impl SearchColumn {
    pub(super) fn parse(value: &str) -> Result<Self, AppError> {
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

    pub(super) fn jira_field(self) -> Option<&'static str> {
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

    pub(super) fn render(self, issue: &SearchIssueOutput) -> String {
        match self {
            Self::Key => super::render::render_key(&issue.key),
            Self::Status => super::render::render_status(&issue.status_name),
            Self::Summary => issue.summary.clone(),
            Self::Components => {
                if issue.components.is_empty() {
                    "-".to_string()
                } else {
                    super::render::render_components(issue)
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

pub(super) fn serialize_human_columns(human_columns: &HumanColumns) -> Option<String> {
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

pub(super) fn has_explicit_search_restriction(intent: &SearchIntent) -> bool {
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

pub(super) fn validate_search_intent(
    intent: &SearchIntent,
    min_limit: u32,
    max_limit: u32,
    default_limit: u32,
) -> Result<(), AppError> {
    validate_limit(intent.limit.unwrap_or(default_limit), min_limit, max_limit)?;
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

fn validate_limit(limit: u32, min_limit: u32, max_limit: u32) -> Result<(), AppError> {
    if (min_limit..=max_limit).contains(&limit) {
        Ok(())
    } else {
        Err(AppError::InvalidSearch {
            reason: format!("--limit must be between {min_limit} and {max_limit}"),
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

#[cfg(test)]
mod tests {
    use crate::cli::SearchArgs;
    use crate::error::AppError;
    use crate::issue_search::tests_support::prepare_with_board_source_for_args;

    #[test]
    fn search_requires_an_explicit_or_configured_restriction() {
        let error = prepare_with_board_source_for_args(
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
        let error = prepare_with_board_source_for_args(
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
        let error = prepare_with_board_source_for_args(
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
            let error = prepare_with_board_source_for_args(
                &args,
                None,
                |_| unreachable!(),
                |_| unreachable!(),
            )
            .unwrap_err();
            assert!(error.to_string().contains("cannot"));
        }
    }

    #[test]
    fn search_rejects_invalid_columns() {
        for columns in ["key,,summary", "key,unknown"] {
            let error = prepare_with_board_source_for_args(
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
            let error = prepare_with_board_source_for_args(
                &args,
                None,
                |_| unreachable!(),
                |_| unreachable!(),
            )
            .unwrap_err();
            assert!(error.to_string().contains("cannot contain empty values"));
        }
    }

    #[test]
    fn search_rejects_invalid_sort_values() {
        for sort in ["", "   ", "updated desc", "updated,created"] {
            let error = prepare_with_board_source_for_args(
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
}
