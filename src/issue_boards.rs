//! Jira board listing domain module.
//!
//! The command adapter enters through `execute`; tests and future callers can use
//! `prepare` when they only need the prepared Jira request.

use crate::cli::BoardsArgs;
use crate::client::{
    JiraClient,
    types::{BoardResponse, ListBoardsRequest, ListBoardsResponse},
};
use crate::error::AppError;
use serde::Serialize;
use std::io::Write;

/// Prepared board listing intent after option normalization.
#[derive(Debug)]
pub struct PreparedBoardList {
    request: ListBoardsRequest,
}

/// jeera-owned boards output; this is the stable interface for JSON and human rendering.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct BoardsOutput {
    boards: Vec<BoardOutput>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct BoardOutput {
    id: u64,
    board_type: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location_name: Option<String>,
}

/// Runs a complete Jira boards operation behind the domain interface.
pub fn execute(client: &JiraClient, args: &BoardsArgs) -> Result<BoardsOutput, AppError> {
    let prepared = prepare(args)?;
    let response = client
        .list_boards(prepared.request())
        .map_err(|source| AppError::ExecuteBoards { source })?;

    Ok(output_from_response(response))
}

/// Builds the Jira request without executing it; useful as the module's narrow test surface.
pub fn prepare(args: &BoardsArgs) -> Result<PreparedBoardList, AppError> {
    validate_project_filter(args.project.as_deref())?;

    Ok(PreparedBoardList {
        request: ListBoardsRequest {
            project_key_or_id: args.project.clone(),
        },
    })
}

/// Human rendering is boards-specific, while JSON rendering stays generic in `render`.
pub fn render_human(mut writer: impl Write, output: &BoardsOutput) -> Result<(), AppError> {
    if output.boards.is_empty() {
        writeln!(writer, "No boards found.").map_err(|source| AppError::RenderOutput { source })?;
        return Ok(());
    }

    let id_width = output
        .boards
        .iter()
        .map(|board| board.id.to_string().len())
        .max()
        .unwrap_or(1);
    let type_width = output
        .boards
        .iter()
        .map(|board| board.board_type.len())
        .max()
        .unwrap_or(1);
    let location_width = output
        .boards
        .iter()
        .map(|board| display_location(board).len())
        .max()
        .unwrap_or(1);

    for board in &output.boards {
        writeln!(
            writer,
            "{id:>id_width$}  {board_type:<type_width$}  {location:<location_width$}  {name}",
            id = board.id,
            board_type = board.board_type,
            location = display_location(board),
            name = board.name,
        )
        .map_err(|source| AppError::RenderOutput { source })?;
    }

    Ok(())
}

impl PreparedBoardList {
    /// Exposes only the transport request; boards preparation remains inside this module.
    pub fn request(&self) -> &ListBoardsRequest {
        &self.request
    }
}

fn validate_project_filter(project: Option<&str>) -> Result<(), AppError> {
    match project {
        Some(project) if project.trim().is_empty() => Err(AppError::InvalidBoards {
            reason: "--project cannot be empty".to_string(),
        }),
        _ => Ok(()),
    }
}

fn output_from_response(response: ListBoardsResponse) -> BoardsOutput {
    BoardsOutput {
        boards: response.values.into_iter().map(BoardOutput::from).collect(),
    }
}

impl From<BoardResponse> for BoardOutput {
    fn from(board: BoardResponse) -> Self {
        let (project_key, location_name) = board
            .location
            .map(|location| {
                let location_name = location
                    .display_name
                    .or(location.name)
                    .or(location.project_name);
                (location.project_key, location_name)
            })
            .unwrap_or((None, None));

        Self {
            id: board.id,
            board_type: board.board_type,
            name: board.name,
            project_key,
            location_name,
        }
    }
}

fn display_location(board: &BoardOutput) -> &str {
    board
        .project_key
        .as_deref()
        .or(board.location_name.as_deref())
        .unwrap_or("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render;
    use std::fs;
    use std::path::Path;

    fn fixture(path: &str) -> String {
        fs::read_to_string(Path::new("tests/fixtures/jira").join(path)).unwrap()
    }

    #[test]
    fn prepare_accepts_missing_project_filter() {
        let prepared = prepare(&BoardsArgs::default()).unwrap();

        assert_eq!(prepared.request().project_key_or_id, None);
    }

    #[test]
    fn prepare_rejects_blank_project_filter() {
        let error = prepare(&BoardsArgs {
            project: Some("   ".to_string()),
            json: false,
        })
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid boards: --project cannot be empty"
        );
    }

    #[test]
    fn prepare_sets_project_filter_when_present() {
        let prepared = prepare(&BoardsArgs {
            project: Some("SAMPLE".to_string()),
            json: false,
        })
        .unwrap();

        assert_eq!(
            prepared.request().project_key_or_id.as_deref(),
            Some("SAMPLE")
        );
    }

    #[test]
    fn deserializes_realistic_board_fixture_into_output() {
        let response: ListBoardsResponse =
            serde_json::from_str(&fixture("boards-basic.json")).unwrap();

        let output = output_from_response(response);

        assert_eq!(output.boards.len(), 4);
        assert_eq!(output.boards[0].id, 215);
        assert_eq!(output.boards[0].project_key.as_deref(), Some("SAMPLE"));
        assert_eq!(
            output.boards[2].location_name.as_deref(),
            Some("Shared Workspace")
        );
        assert_eq!(output.boards[3].project_key, None);
    }

    #[test]
    fn render_human_aligns_location_and_name_columns() {
        let response: ListBoardsResponse =
            serde_json::from_str(&fixture("boards-basic.json")).unwrap();
        let output = output_from_response(response);
        let mut rendered = Vec::new();

        render_human(&mut rendered, &output).unwrap();

        assert_eq!(
            String::from_utf8(rendered).unwrap(),
            concat!(
                "215  kanban  SAMPLE            SAMPLE Kanban Board\n",
                "212  kanban  SAMPLE            Release Dashboard\n",
                "314  scrum   Shared Workspace  Platform Sprint\n",
                "999  simple  -                 Sandbox Board\n",
            )
        );
    }

    #[test]
    fn render_human_shows_empty_state() {
        let output = BoardsOutput { boards: Vec::new() };
        let mut rendered = Vec::new();

        render_human(&mut rendered, &output).unwrap();

        assert_eq!(String::from_utf8(rendered).unwrap(), "No boards found.\n");
    }

    #[test]
    fn render_json_emits_stable_jeera_owned_schema() {
        let response: ListBoardsResponse =
            serde_json::from_str(&fixture("boards-basic.json")).unwrap();
        let output = output_from_response(response);
        let mut rendered = Vec::new();

        render::render_json(&mut rendered, &output).unwrap();

        let rendered = String::from_utf8(rendered).unwrap();
        assert!(rendered.contains("\"boards\": ["));
        assert!(rendered.contains("\"project_key\": \"SAMPLE\""));
        assert!(rendered.contains("\"location_name\": \"Shared Workspace\""));
    }
}
