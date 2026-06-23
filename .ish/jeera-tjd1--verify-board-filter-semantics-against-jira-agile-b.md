---
# jeera-tjd1
title: Verify board filter semantics against Jira Agile board results
status: completed
type: task
priority: normal
tags:
- search
created_at: 2026-06-23T16:47:55.848997Z
updated_at: 2026-06-23T20:01:49.053172Z
parent: jeera-ejui
blocked_by:
- jeera-suhm
---

## Context
To make `--board` a normal filter, current search resolves board configuration and builds JQL like `filter = <id> AND (<subQuery>)`. This uses the unified `/rest/api/3/search/jql` endpoint rather than `/rest/agile/1.0/board/{id}/issue`.

This appears to work for board 215, but it should be verified against Jira's Agile board issue endpoint for representative boards before relying on it broadly.

## Dependencies
Blocked by `jeera-suhm` so the final generated JQL can be inspected easily while comparing results.

## Work
- Compare result sets for the unified JQL approach and Agile board issue endpoint for at least board 215 and one other GCCDEV board if accessible.
- Test with and without extra filters such as component/status/assignee.
- Document any known differences, especially around board subQuery, rank sorting, closed issues, epics, and permissions.
- If differences matter, adjust implementation or create follow-up work with exact reproduction details.
- Add automated client/unit coverage for board configuration handling if missing, but do not require live Jira integration tests in CI.

## Verification
- `cargo fmt`
- `cargo test`
- `cargo clippy -- -D warnings`
- Manual comparison notes recorded in this ish or implementation summary, including commands used and whether result keys match for sampled queries.



## Implementation Notes
- Added Jira client transport support for `GET /rest/agile/1.0/board/{id}/issue` via `ListBoardIssuesRequest`, `ListBoardIssuesResponse`, and `JiraClient::list_board_issues(...)` plus request-shape coverage. This is not wired into the CLI yet; it exists so future workers can compare unified search behavior against the Agile endpoint without re-adding transport code.
- No product behavior changed in this ish. The current unified `--board` implementation remains `filter = <board filter id> AND (<board subQuery>)` sent through `/rest/api/3/search/jql`.

## Manual Comparison Notes
Commands and requests used against live Jira:
- `cargo run -- boards --project GCCDEV --json` to identify sample boards.
- `cargo run -- search --board 215 --limit 5 --debug-jql` to capture the exact unified JQL.
- Direct Jira comparisons between `/rest/api/3/search/jql` and `/rest/agile/1.0/board/{id}/issue` for sampled boards and filters.

Results:
- Board 215: `filter = 10492 AND (fixVersion in unreleasedVersions() OR fixVersion is EMPTY) ORDER BY Rank ASC` returned the exact same first 10 keys, in the same order, as `GET /rest/agile/1.0/board/215/issue?maxResults=10`.
- Board 215 with extra filters also matched exactly against the Agile endpoint when the same extra JQL was supplied there:
  - `component = "QQMS"`
  - `statusCategory != Done`
- Board 211 (`Adapt Beta Feedback`) also matched exactly, both with no extra filter and with `assignee = currentUser()`.
- Observed important difference: the Agile endpoint defaults to board rank order, while `jeera search` currently defaults to `ORDER BY updated DESC`. When unified search is run with `--sort Rank --asc`, the sampled keys match the Agile endpoint; with the current default updated sort, they intentionally differ in ordering and visible issues on the first page.
- No sampled differences were observed around board subQuery application, permissions, closed-issue inclusion, or supplemental filter composition once sort order was aligned.

## Verification Results
- Passed: `cargo fmt`
- Passed: `cargo test`
- Passed: `cargo clippy -- -D warnings`
- Passed: `ish check`
