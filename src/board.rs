//! Shared Jira board selection and lookup helpers.
//!
//! Board-specific domain behavior stays in the domain module that needs it
//! (for example, search's board filter JQL). This module only owns common board
//! identity and location concerns.

use crate::client::types::BoardResponse;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardSelector {
    Id(u64),
    Name(String),
}

impl BoardSelector {
    pub fn to_cli_value(&self) -> String {
        match self {
            Self::Id(board_id) => board_id.to_string(),
            Self::Name(board_name) => board_name.clone(),
        }
    }
}

pub fn parse_board_selector(board: &str, field_name: &str) -> Result<BoardSelector, String> {
    if board.is_empty() {
        return Err(format!("--{field_name} cannot be empty"));
    }

    match board.parse::<u64>() {
        Ok(board_id) => Ok(BoardSelector::Id(board_id)),
        Err(_) => Ok(BoardSelector::Name(board.to_string())),
    }
}

pub fn find_board_id_by_name(boards: &[BoardResponse], board_name: &str) -> Result<u64, String> {
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
        [] => Err(format!(
            "no Jira board named {board_name:?} found; try `jeera boards` to discover available boards or pass a numeric --board ID"
        )),
        [board] => Ok(board.id),
        boards => Err(format!(
            "board name {board_name:?} is ambiguous; matching board ids: {}. Try `jeera boards` or pass a numeric --board ID",
            boards
                .iter()
                .map(|board| board.id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

pub fn find_board_project_key(boards: &[BoardResponse], board_id: u64) -> Result<String, String> {
    let board = boards
        .iter()
        .find(|board| board.id == board_id)
        .ok_or_else(|| format!("no Jira board with id {board_id} found; try `jeera boards` to discover available boards or pass --project explicitly"))?;

    board
        .location
        .as_ref()
        .and_then(|location| location.project_key.as_deref())
        .filter(|project_key| !project_key.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            format!("board {board_id} does not expose a project key; pass --project explicitly")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::types::{BoardLocationResponse, BoardResponse};

    fn board_response(id: u64, name: &str, project_key: Option<&str>) -> BoardResponse {
        BoardResponse {
            id,
            name: name.to_string(),
            board_type: "kanban".to_string(),
            location: project_key.map(|project_key| BoardLocationResponse {
                project_id: Some(10000),
                project_key: Some(project_key.to_string()),
                project_name: Some("Sample".to_string()),
                display_name: Some("Sample".to_string()),
                name: Some("Sample".to_string()),
            }),
        }
    }

    #[test]
    fn board_name_matching_is_case_insensitive_when_needed() {
        let boards = vec![board_response(215, "SAMPLE Kanban Board", Some("SAMPLE"))];

        assert_eq!(
            find_board_id_by_name(&boards, "sample kanban board").unwrap(),
            215
        );
    }

    #[test]
    fn unknown_board_name_is_reported_clearly() {
        let boards = vec![board_response(215, "SAMPLE Kanban Board", Some("SAMPLE"))];

        let error = find_board_id_by_name(&boards, "Missing Board").unwrap_err();

        assert_eq!(
            error,
            "no Jira board named \"Missing Board\" found; try `jeera boards` to discover available boards or pass a numeric --board ID"
        );
    }

    #[test]
    fn ambiguous_board_name_is_reported_clearly() {
        let boards = vec![
            board_response(215, "Team Board", Some("SAMPLE")),
            board_response(314, "Team Board", Some("SAMPLE")),
        ];

        let error = find_board_id_by_name(&boards, "Team Board").unwrap_err();

        assert_eq!(
            error,
            "board name \"Team Board\" is ambiguous; matching board ids: 215, 314. Try `jeera boards` or pass a numeric --board ID"
        );
    }

    #[test]
    fn finds_project_key_for_board() {
        let boards = vec![board_response(215, "SAMPLE Kanban Board", Some("SAMPLE"))];

        assert_eq!(find_board_project_key(&boards, 215).unwrap(), "SAMPLE");
    }

    #[test]
    fn missing_board_project_key_is_reported_clearly() {
        let boards = vec![board_response(215, "SAMPLE Kanban Board", None)];

        assert_eq!(
            find_board_project_key(&boards, 215).unwrap_err(),
            "board 215 does not expose a project key; pass --project explicitly"
        );
    }
}
