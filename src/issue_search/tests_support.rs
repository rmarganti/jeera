use super::prepare::prepare_with_board_source;
use super::{AppError, SearchIntent};
use crate::cli::SearchArgs;
use crate::client::types::BoardResponse;
use crate::client::types::SearchIssuesResponse;
use crate::issue_search::board::BoardJqlFilter;
use crate::issue_search::output::{SearchIssueFields, output_from_search_response};
use crate::issue_search::prepare::PreparedIssueSearch;
use crate::issue_search::{SearchColumn, SearchOutput};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::thread;
use std::time::Duration;
use url::Url;

pub(super) fn fixture(path: &str) -> String {
    fs::read_to_string(Path::new("tests/fixtures/jira").join(path)).unwrap()
}

pub(super) fn parse_fixture(path: &str) -> SearchOutput {
    let response: SearchIssuesResponse<SearchIssueFields> =
        serde_json::from_str(&fixture(path)).unwrap();
    output_from_search_response(response)
}

pub(super) fn spawn_server(
    status_line: &str,
    body: &str,
) -> (Url, std::sync::mpsc::Receiver<String>) {
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

pub(super) fn prepare_with_board_source_for_args<R, F>(
    args: &SearchArgs,
    default_board_id: Option<u64>,
    resolve_board_name: R,
    load_board_filter: F,
) -> Result<PreparedIssueSearch, AppError>
where
    R: FnMut(&str) -> Result<u64, AppError>,
    F: FnOnce(u64) -> Result<BoardJqlFilter, AppError>,
{
    prepare_with_board_source(
        &SearchIntent::try_from(args)?,
        default_board_id,
        resolve_board_name,
        load_board_filter,
        super::SEARCH_MIN_LIMIT,
        super::SEARCH_MAX_LIMIT,
        super::DEFAULT_SEARCH_LIMIT,
    )
}

pub(super) fn prepare_without_board(args: &SearchArgs) -> PreparedIssueSearch {
    prepare_with_board_source_for_args(args, None, |_| unreachable!(), |_| unreachable!()).unwrap()
}

pub(super) fn render_human(
    writer: impl Write,
    output: &SearchOutput,
    columns: &[SearchColumn],
    next_page_command: Option<&str>,
) -> Result<(), AppError> {
    super::render::render_human_output(writer, output, columns, next_page_command)
}

pub(super) fn board_filter(filter_id: u64, sub_query: &str) -> BoardJqlFilter {
    BoardJqlFilter {
        filter_id,
        sub_query: Some(sub_query.to_string()),
    }
}

pub(super) fn board_response(id: u64, name: &str, board_type: &str) -> BoardResponse {
    BoardResponse {
        id,
        name: name.to_string(),
        board_type: board_type.to_string(),
        location: None,
    }
}
