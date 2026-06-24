use crate::client::{
    JiraClient,
    types::{BoardResponse, GetBoardConfigurationRequest, ListBoardsRequest},
};
use crate::error::AppError;

/// Domain form of Jira board configuration, before it becomes JQL clauses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoardJqlFilter {
    pub(crate) filter_id: u64,
    pub(crate) sub_query: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BoardSelector {
    Id(u64),
    Name(String),
}

impl BoardSelector {
    pub(super) fn to_cli_value(&self) -> String {
        match self {
            Self::Id(board_id) => board_id.to_string(),
            Self::Name(board_name) => board_name.clone(),
        }
    }
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

pub(super) fn parse_board_selector(board: &str) -> Result<BoardSelector, AppError> {
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

pub(super) fn resolve_board_name(client: &JiraClient, board_name: &str) -> Result<u64, AppError> {
    let response = client
        .list_boards(&ListBoardsRequest::default())
        .map_err(|source| AppError::ExecuteBoards { source })?;

    find_board_id_by_name(&response.values, board_name)
}

pub(crate) fn find_board_id_by_name(
    boards: &[BoardResponse],
    board_name: &str,
) -> Result<u64, AppError> {
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
