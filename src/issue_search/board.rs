use crate::board;
pub(super) use crate::board::BoardSelector;
use crate::client::{
    JiraClient,
    types::{GetBoardConfigurationRequest, ListBoardsRequest},
};
use crate::error::AppError;

/// Domain form of Jira board configuration, before it becomes JQL clauses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoardJqlFilter {
    pub(crate) filter_id: u64,
    pub(crate) sub_query: Option<String>,
}

pub(super) fn resolve_board_id<R>(
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

pub(super) fn parse_board_selector(value: &str) -> Result<BoardSelector, AppError> {
    board::parse_board_selector(value, "board").map_err(|reason| AppError::InvalidSearch { reason })
}

pub(super) fn resolve_board_name(client: &JiraClient, board_name: &str) -> Result<u64, AppError> {
    let response = client
        .list_boards(&ListBoardsRequest::default())
        .map_err(|source| AppError::ExecuteBoards { source })?;

    board::find_board_id_by_name(&response.values, board_name)
        .map_err(|reason| AppError::InvalidSearch { reason })
}

pub(super) fn board_filter(client: &JiraClient, board_id: u64) -> Result<BoardJqlFilter, AppError> {
    let configuration = client
        .get_board_configuration(&GetBoardConfigurationRequest { board_id })
        .map_err(|source| AppError::PrepareBoardSearch { board_id, source })?;

    Ok(BoardJqlFilter {
        filter_id: parse_board_filter_id(board_id, &configuration.filter.id)?,
        sub_query: Some(configuration.sub_query.query),
    })
}

pub(crate) fn parse_board_filter_id(board_id: u64, filter_id: &str) -> Result<u64, AppError> {
    filter_id
        .parse()
        .map_err(|source| AppError::InvalidBoardFilterId {
            board_id,
            filter_id: filter_id.to_string(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::SearchArgs;
    use crate::issue_search::tests_support::{
        board_filter as test_board_filter, board_response, prepare_with_board_source_for_args,
    };

    #[test]
    fn numeric_board_reference_bypasses_name_resolution() {
        let prepared = prepare_with_board_source_for_args(
            &SearchArgs {
                board: Some("215".to_string()),
                ..Default::default()
            },
            None,
            |_| panic!("numeric board ids should not invoke board-name resolution"),
            |board_id| {
                assert_eq!(board_id, 215);
                Ok(test_board_filter(10492, "fixVersion is EMPTY"))
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
        let prepared = prepare_with_board_source_for_args(
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
                Ok(test_board_filter(10492, "fixVersion is EMPTY"))
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
        let boards = vec![board_response(215, "SAMPLE Kanban Board", "kanban")];

        assert_eq!(
            board::find_board_id_by_name(&boards, "sample kanban board").unwrap(),
            215
        );
    }

    #[test]
    fn unknown_board_name_is_reported_clearly() {
        let boards = vec![board_response(215, "SAMPLE Kanban Board", "kanban")];

        let error = board::find_board_id_by_name(&boards, "Missing Board").unwrap_err();

        assert_eq!(
            error,
            "no Jira board named \"Missing Board\" found; try `jeera boards` to discover available boards or pass a numeric --board ID"
        );
    }

    #[test]
    fn ambiguous_board_name_is_reported_clearly() {
        let boards = vec![
            board_response(215, "Team Board", "kanban"),
            board_response(314, "Team Board", "scrum"),
        ];

        let error = board::find_board_id_by_name(&boards, "Team Board").unwrap_err();

        assert_eq!(
            error,
            "board name \"Team Board\" is ambiguous; matching board ids: 215, 314. Try `jeera boards` or pass a numeric --board ID"
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
}
