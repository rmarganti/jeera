# Context

## Domain vocabulary

- **Jira issue** — an issue the user can list or view from Jira.
- **Jira issue search** — the domain module that turns normalized search intent into a Jira issue search request and maps Jira search responses into jeera-owned output.
- **Jira issue view** — the domain module that turns a Jira issue key into detailed issue output.
- **Jira issue creation** — the domain module that turns CLI-first create inputs into a validated Jira create issue request and maps Jira's create response into jeera-owned output.
- **Command adapter** — a thin CLI-facing adapter that parses command arguments, calls a domain module, and chooses output mode.
- **Board selector** — a numeric board id or exact board name used to identify a Jira board consistently across commands.
- **Board filter** — a Jira board-derived filter and optional sub-query used to restrict Jira issue search.
- **Project resolution** — choosing the Jira project for an operation from an explicit `--project`, an explicit board, or the configured `default_board_id`.
- **Search intent** — normalized search request derived from user inputs such as CLI/search args.
- **Effective search intent** — search intent after applying a saved search profile and CLI overrides.
- **JQL** — Jira Query Language, the SQL-like query language Jira's API uses for filtering, etc.
- **Issue body** — long-form text supplied by `--body` or `--body-file` and converted to Atlassian Document Format for Jira descriptions.
- **Create metadata** — Jira project/issue type field metadata that tells jeera which fields are available or required when creating an issue.
- **Edit metadata** — Jira issue field metadata that tells jeera which fields are editable on an existing issue.
- **Transition metadata** — Jira workflow transition data that tells jeera which status transitions are currently available for an issue.
