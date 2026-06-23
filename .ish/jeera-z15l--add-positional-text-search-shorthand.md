---
# jeera-z15l
title: Add positional text search shorthand
status: completed
type: feature
priority: normal
tags:
- search
created_at: 2026-06-23T16:46:49.340787Z
updated_at: 2026-06-23T19:46:38.098611Z
parent: jeera-ejui
blocked_by:
- jeera-bcud
---

## Context
Today users must write `jeera search --text reporting` to search Jira text. For daily use, a concise shorthand like `jeera search reporting` is easier. Search must still avoid implicit filters: a positional query is an explicit user-provided restriction.

## Dependencies
Blocked by `jeera-bcud` because positional input should participate in the same validation and JQL-building rules as the existing `--text` filter.

## Work
- Add an optional positional query argument to `SearchArgs`, e.g. `pub query: Option<String>`.
- Map the positional query to the same JQL as `--text`: `text ~ "..."`.
- Decide conflict/combination behavior with `--text`. Preferred: allow both only if they are combined intentionally with `AND`; otherwise reject with a clear error. Document the chosen behavior in tests/help.
- Ensure `jeera search reporting` is considered an explicit search restriction and does not trigger the no-filter error.
- Add tests for positional-only, positional plus board/default board, and positional plus `--jql` behavior.

## Verification
- `cargo fmt`
- `cargo test`
- `cargo clippy -- -D warnings`
- Manual checks:
  - `cargo run -- search reporting --limit 5`
  - `cargo run -- search reporting --board 215 --limit 5`
  - `cargo run -- search --text reporting --limit 5` still works.

## Implementation Notes
- Added an optional positional `QUERY` argument to `jeera search` via `SearchArgs::query`.
- Treated the positional query as the same JQL primitive as `--text`: `text ~ "..."`.
- Counted the positional query as an explicit search restriction so it works without `default_board_id` and does not trip the no-filter validation.
- Chose to combine positional `QUERY` and `--text` with `AND` by emitting two `text ~ ...` clauses; this keeps both restrictions explicit and matches the ish preference.
- Added clap parsing tests plus search-preparation tests for positional-only, positional+default-board, positional+raw-`--jql`, and positional+`--text` behavior.

## Verification Results
- Passed: `cargo fmt`
- Passed: `cargo test`
- Passed: `cargo clippy -- -D warnings`
- Passed: `ish check`
- Not run here: live manual Jira checks require local Jira access/config.
