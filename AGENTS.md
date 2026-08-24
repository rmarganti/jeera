# AGENTS

## Project overview

`jeera` is a read-only Jira CLI written in Rust.

Its primary goal is to let a user list and view Jira issues they have access to.

## References

Jira's OpenAPI spec is available at `openapi/jira-v3-openapi.json`.

## Tooling notes

- Run `cargo fmt` after code changes.
- Run `cargo clippy -- -D warnings` before considering work complete.
- Run `cargo test` when changing behavior, parsing, rendering, or client code.
