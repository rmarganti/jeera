use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "jeera")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Boards(BoardsArgs),
    Search(Box<SearchArgs>),
    Show(ShowArgs),
}

#[derive(Debug, Args, Default)]
pub struct BoardsArgs {
    #[arg(long)]
    pub project: Option<String>,

    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args, Clone, Default)]
pub struct SearchArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub profile: Option<String>,

    #[arg(
        value_name = "QUERY",
        help = "Search Jira text with a concise positional query"
    )]
    pub query: Option<String>,

    #[arg(
        long,
        help = "Combine raw JQL with any structured filters instead of replacing them"
    )]
    pub jql: Option<String>,

    #[arg(long, value_name = "ID|NAME", help = "Board id or exact board name")]
    pub board: Option<String>,

    #[arg(long)]
    pub project: Option<String>,

    #[arg(long, conflicts_with = "unassigned")]
    pub assignee: Option<String>,

    #[arg(long)]
    pub unassigned: bool,

    #[arg(long)]
    pub reporter: Option<String>,

    #[arg(long)]
    pub status: Vec<String>,

    #[arg(long = "status-category")]
    pub status_category: Option<String>,

    #[arg(long = "type", alias = "issue-type")]
    pub issue_type: Vec<String>,

    #[arg(long)]
    pub component: Vec<String>,

    #[arg(long)]
    pub label: Vec<String>,

    #[arg(long)]
    pub text: Option<String>,

    #[arg(long)]
    pub open: bool,

    #[arg(long)]
    pub limit: Option<u32>,

    #[arg(long)]
    pub next_page_token: Option<String>,

    #[arg(
        long,
        value_name = "COLS",
        help = "Comma-separated human output columns: key,status,summary,components,type,assignee,priority,updated"
    )]
    pub columns: Option<String>,

    #[arg(
        long,
        help = "Print the final JQL to stderr before executing the search"
    )]
    pub debug_jql: bool,

    #[arg(
        long,
        help = "Sort by Jira field or alias (rank, updated, created, priority)"
    )]
    pub sort: Option<String>,

    #[arg(long, conflicts_with = "desc")]
    pub asc: bool,

    #[arg(
        long,
        help = "Explicitly request descending sort order; without --asc/--desc, board searches default to Rank ASC and other searches default to updated DESC"
    )]
    pub desc: bool,
}

#[derive(Debug, Args, Default)]
pub struct ShowArgs {
    pub issue_key: String,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub comments: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn search_accepts_a_positional_query() {
        let cli = Cli::parse_from(["jeera", "search", "reporting"]);

        match cli.command {
            Command::Search(args) => {
                assert_eq!(args.query.as_deref(), Some("reporting"));
                assert_eq!(args.profile, None);
                assert_eq!(args.text, None);
            }
            other => panic!("expected search command, got {other:?}"),
        }
    }

    #[test]
    fn search_accepts_a_positional_query_with_flags() {
        let cli = Cli::parse_from(["jeera", "search", "--board", "215", "reporting"]);

        match cli.command {
            Command::Search(args) => {
                assert_eq!(args.board.as_deref(), Some("215"));
                assert_eq!(args.query.as_deref(), Some("reporting"));
            }
            other => panic!("expected search command, got {other:?}"),
        }
    }

    #[test]
    fn search_accepts_named_board_references() {
        let cli = Cli::parse_from([
            "jeera",
            "search",
            "--board",
            "SAMPLE Kanban Board",
            "reporting",
        ]);

        match cli.command {
            Command::Search(args) => {
                assert_eq!(args.board.as_deref(), Some("SAMPLE Kanban Board"));
                assert_eq!(args.query.as_deref(), Some("reporting"));
            }
            other => panic!("expected search command, got {other:?}"),
        }
    }

    #[test]
    fn search_accepts_configurable_columns() {
        let cli = Cli::parse_from([
            "jeera",
            "search",
            "--project",
            "SAMPLE",
            "--columns",
            "key,type,status,assignee,updated,summary",
        ]);

        match cli.command {
            Command::Search(args) => {
                assert_eq!(args.project.as_deref(), Some("SAMPLE"));
                assert_eq!(
                    args.columns.as_deref(),
                    Some("key,type,status,assignee,updated,summary")
                );
            }
            other => panic!("expected search command, got {other:?}"),
        }
    }

    #[test]
    fn search_accepts_optional_sort_and_direction_flags() {
        let cli = Cli::parse_from([
            "jeera",
            "search",
            "--profile",
            "qqms",
            "--sort",
            "rank",
            "--desc",
            "demo",
        ]);

        match cli.command {
            Command::Search(args) => {
                assert_eq!(args.profile.as_deref(), Some("qqms"));
                assert_eq!(args.sort.as_deref(), Some("rank"));
                assert!(args.desc);
                assert_eq!(args.query.as_deref(), Some("demo"));
            }
            other => panic!("expected search command, got {other:?}"),
        }
    }
}
