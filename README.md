# jeera

Jira CLI for listing boards, searching issues, viewing issue details, inspecting metadata, and explicitly enabled issue mutations.

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
  "mutations_enabled": false,
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
- `mutations_enabled`: optional, defaults to `false`; a human must manually set this to `true` before mutation commands can run
- `default_board_id`: optional board id used by commands such as `jeera search`, `jeera show-create-meta`, and `jeera create` when `--board`/`--project` is omitted
- `searches`: optional map of saved search profiles for `jeera search --profile <NAME>`

### Saved search profiles

Profiles let you preconfigure common `search` filters in config and then override or extend them from the CLI.

```json
{
  "base_url": "https://your-domain.atlassian.net",
  "default_board_id": 215,
  "searches": {
    "qqms": {
      "project": "GCCDEV",
      "component": ["QQMS"],
      "open": true,
      "sort": "rank",
      "asc": true,
      "limit": 25
    }
  },
  "auth": {
    "type": "bearer",
    "token": "<token>"
  }
}
```

Supported profile fields mirror `jeera search`: `board`, `jql`, `project`, `assignee`, `unassigned`, `reporter`, `status`, `status_category`, `issue_type`, `component`, `label`, `text`, `open`, `limit`, `sort`, `asc`, `desc`.

Merge behavior:

- Profile values load first.
- Explicit CLI scalar flags override profile values.
- Repeated CLI flags like `--status`, `--component`, `--label`, and `--type` append to profile values.
- `default_board_id` still applies if neither the profile nor the CLI specifies a board.

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
- `--profile <NAME>` load a saved search profile from config
- `--jql <JQL>` combine raw JQL with structured filters
- `--board <ID|NAME>` use a board filter by numeric id or exact board name; falls back to `default_board_id`
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
- `--sort <FIELD>` optional Jira sort field or alias (`rank`, `updated`, `created`, `priority`)
- `--asc`
- `--desc`

Notes:

- Human output is colorized; `--json` output is not.
- Board-scoped searches default to `ORDER BY Rank ASC`; other searches default to `ORDER BY updated DESC`.
- `--sort rank` maps to Jira `Rank` and defaults to ascending order unless you pass `--desc`.
- When more results are available, human output includes both the next page token and a copy/pasteable next-page command.
- `--columns` affects human output only; JSON remains a stable structured schema.

Examples:

```sh
jeera search reporting
jeera search --profile qqms
jeera search --profile qqms --status 'In Progress'
jeera search --assignee me --open
jeera search --board 215 --columns key,type,status,assignee,updated,summary --limit 5
jeera search --board 'GCCDEV Kanban Board' --limit 5
jeera search --board 215 --sort rank --desc --limit 5
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

### `jeera create`

Create a Jira issue. Mutation commands are disabled by default. Before this command can run, a human must manually set `"mutations_enabled": true` in the jeera config. Creation is CLI-first: use flags for fields, and use `--body-file` only when the description/body is long.

```sh
jeera create --project GCCDEV --type Task --summary "Add homes intake validation" [OPTIONS]
jeera create --board 215 --type Spike --summary "Discover source" --body-file ./body.md
```

Options:

- `--project <KEY>` Jira project key; conflicts with `--board`
- `--board <ID|NAME>` derive the project from a board; falls back to `default_board_id` when both `--project` and `--board` are omitted
- `--type <TYPE>` / `--issue-type <TYPE>` issue type name
- `--summary <TEXT>` issue summary
- `--body <TEXT>` issue description/body text
- `--body-file <PATH|->` read issue description/body text from a file or stdin
- `--component <COMPONENT>` repeatable
- `--label <LABEL>` repeatable
- `--field <KEY=VALUE>` repeatable string-valued field escape hatch for required Jira fields that do not yet have dedicated flags
- `--dry-run` validate and print what would be created without creating the issue
- `--json`

Before creating, jeera loads Jira create metadata for the resolved project and issue type, then rejects missing required create fields with a clear error.

Examples:

```sh
jeera create --project GCCDEV --type Task --summary "Add homes intake validation" --body-file ./ticket-body.md --component Homes --label homes-2 --dry-run
jeera create --board 'GCCDEV Kanban Board' --type Spike --summary "Discover canonical market source" --body "Identify the canonical source."
```

### `jeera update`

Update an existing Jira issue through friendly field flags. Mutation commands are disabled by default and require `"mutations_enabled": true` in the jeera config. Omitted fields remain unchanged; repeated component and label values replace the corresponding collection.

```sh
jeera update GCCDEV-123 --summary "Clarify homes intake validation"
jeera update GCCDEV-123 --body-file ./ticket-body.md --component Homes --label homes-2
jeera update GCCDEV-123 --clear-body --clear-labels
```

Options:

- `--summary <TEXT>` replace the issue summary
- `--body <TEXT>` replace the description/body
- `--body-file <PATH|->` replace the description/body from a file or stdin
- `--component <COMPONENT>` repeatable; replace all components
- `--label <LABEL>` repeatable; replace all labels
- `--clear-body`, `--clear-components`, `--clear-labels` explicitly clear values
- `--field <KEY=VALUE>` repeatable string-valued field escape hatch
- `--dry-run` validate against Jira edit metadata without updating the issue
- `--json`

Before updating, jeera loads edit metadata and verifies that every requested field supports Jira's `set` operation.

### `jeera show-create-meta`

Show Jira create metadata, including available issue types and required fields. If `--project` is omitted, jeera can derive the project from `--board` or the configured `default_board_id`.

```sh
jeera show-create-meta [--project GCCDEV | --board 215] [--type Task] [--json]
jeera show-create-meta
```

### `jeera show-edit-meta`

Show editable fields for an existing issue.

```sh
jeera show-edit-meta GCCDEV-123 [--json]
```

### `jeera show-transitions`

Show available workflow transitions for an existing issue.

```sh
jeera show-transitions GCCDEV-123 [--json]
```
