# jeera

Read-only Jira CLI for listing and viewing issues.

## Configuration

`jeera` reads JSON config from:

1. `$JEERA_CONFIG`
2. `$XDG_CONFIG_HOME/jeera/settings.json`
3. `~/.config/jeera/settings.json`

Basic auth:

```json
{
  "base_url": "https://your-domain.atlassian.net",
  "http_timeout_seconds": 30,
  "default_board_id": 123,
  "auth": {
    "type": "basic",
    "email": "you@example.com",
    "api_token": "<jira-api-token>"
  }
}
```

Bearer auth:

```json
{
  "base_url": "https://your-domain.atlassian.net",
  "auth": {
    "type": "bearer",
    "token": "<token>"
  }
}
```

Optional fields: `http_timeout_seconds` (default `30`), `default_board_id`.

## Commands

### Search issues

```sh
jeera search [options]
```

Requires at least one restriction, or a configured `default_board_id`.

Common options:

- `--jql <JQL>` raw JQL, combinable with other filters
- `--board <ID>` board filter; falls back to `default_board_id`
- `--project <KEY>`
- `--assignee <USER|me>` / `--unassigned`
- `--reporter <USER|me>`
- `--status <STATUS>` repeatable
- `--status-category <CATEGORY>`
- `--type <TYPE>` / `--issue-type <TYPE>` repeatable
- `--component <COMPONENT>` repeatable
- `--label <LABEL>` repeatable
- `--text <TEXT>`
- `--open` excludes Done status category
- `--limit <N>` default `50`
- `--next-page-token <TOKEN>`
- `--sort <FIELD>` default `updated`
- `--asc` / `--desc` (default desc)
- `--json`

Examples:

```sh
jeera search --assignee me --open
jeera search --project ENG --status "In Progress" --limit 25
jeera search --jql 'project = ENG ORDER BY priority DESC' --json
```

### Show an issue

```sh
jeera show <ISSUE-KEY> [--comments] [--json]
```

Examples:

```sh
jeera show ENG-123
jeera show ENG-123 --comments --json
```
