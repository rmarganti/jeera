# Context

## Domain vocabulary

- **Jira issue** — an issue the user can list or view from Jira.
- **Jira issue search** — the domain module that turns user search intent into a Jira issue search request and maps Jira search responses into jeera-owned output.
- **Jira issue view** — the domain module that turns a Jira issue key into detailed issue output.
- **Command adapter** — a thin CLI-facing adapter that parses command arguments, calls a domain module, and chooses output mode.
- **Board filter** — a Jira board-derived filter and optional sub-query used to restrict Jira issue search.
- **JQL** — Jira Query Language assembled by jeera from structured search intent and optional raw clauses.
