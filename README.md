# jeera

Read-only Jira CLI for listing boards, searching issues, and viewing issue details.

## Configuration

`jeera` reads JSON config from:

1. `$JEERA_CONFIG`
2. `$XDG_CONFIG_HOME/jeera/settings.json`
3. `~/.config/jeera/settings.json`

### Basic auth

```json
{
  "base_url": "https://your-domain.atlassian.net",
  "http_timeout_seconds": 30,
  "default_board_id": 215,
  "auth": {
    "type": "basic",
    "email": "you@example.com",
    "api_token": "<jira-api-token>"
  }
}
```

### Bearer auth

```json
{
  "base_url": "https://your-domain.atlassian.net",
  "auth": {
    "type": "bearer",
    "token": "<token>"
  }
}
```

### Config fields

- `base_url`: absolute `http` or `https` Jira base URL
- `auth`: `basic` or `bearer`
- `http_timeout_seconds`: optional, defaults to `30`
- `default_board_id`: optional board id used by `jeera search` when `--board` is omitted

## Commands

### `jeera boards`

List Jira Agile boards.

```sh
jeera boards [--project <KEY>] [--json]
```

Examples:

```sh
jeera boards
jeera boards --project GCCDEV
jeera boards --project GCCDEV --json
```

Human output includes board id, board type, and the best available project/location label.

### `jeera search`

Search Jira issues.

```sh
jeera search [OPTIONS] [QUERY]
```

`QUERY` is a positional text-search shorthand equivalent to adding a text restriction.

Search requires at least one explicit restriction, or a configured `default_board_id`.

Options:

- `--json`
- `--jql <JQL>` combine raw JQL with structured filters
- `--board <ID>` use a board filter; falls back to `default_board_id`
- `--project <KEY>`
- `--assignee <USER|me>`
- `--unassigned`
- `--reporter <USER|me>`
- `--status <STATUS>` repeatable
- `--status-category <CATEGORY>`
- `--type <TYPE>` / `--issue-type <TYPE>` repeatable
- `--component <COMPONENT>` repeatable
- `--label <LABEL>` repeatable
- `--text <TEXT>`
- `--open`
- `--limit <N>` defaults to `50`, valid range `1..=100`
- `--next-page-token <TOKEN>` resume a paginated search
- `--columns <COLS>` customize human output columns with `key,status,summary,components,type,assignee,priority,updated`
- `--debug-jql` print the final JQL to stderr before executing
- `--sort <FIELD>` defaults to `updated`
- `--asc`
- `--desc` explicit no-op for the default descending order

Notes:

- Human output is colorized; `--json` output is not.
- When more results are available, human output includes both the next page token and a copy/pasteable next-page command.
- `--columns` affects human output only; JSON remains a stable structured schema.

Examples:

```sh
jeera search reporting
jeera search --assignee me --open
jeera search --board 215 --columns key,type,status,assignee,updated,summary --limit 5
jeera search --project GCCDEV --component QQMS --debug-jql
jeera search --jql 'project = GCCDEV' --status 'In Progress' --json
```

### `jeera show`

Show one issue in detail.

```sh
jeera show <ISSUE_KEY> [--comments] [--json]
```

Examples:

```sh
jeera show GCCDEV-123
jeera show GCCDEV-123 --comments
jeera show GCCDEV-123 --comments --json
```

Human output includes summary, status, type, priority, assignee, reporter, created/updated timestamps, components, description, and optional comments.
