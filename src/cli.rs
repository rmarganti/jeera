use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "jeera")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Search(Box<SearchArgs>),
    Show(ShowArgs),
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub jql: Option<String>,

    #[arg(long)]
    pub board: Option<u64>,

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

    #[arg(long, default_value_t = 50)]
    pub limit: u32,

    #[arg(long)]
    pub next_page_token: Option<String>,

    #[arg(long, default_value = "updated")]
    pub sort: String,

    #[arg(long, conflicts_with = "desc")]
    pub asc: bool,

    #[arg(
        long,
        help = "Explicitly request descending sort order (the default if --asc is not set)"
    )]
    pub desc: bool,
}

// Keep this in sync with clap defaults above (`default_value_t` / `default_value`).
// Tests construct SearchArgs directly, so derive(Default) would not honor clap's runtime defaults.
impl Default for SearchArgs {
    fn default() -> Self {
        Self {
            json: false,
            jql: None,
            board: None,
            project: None,
            assignee: None,
            unassigned: false,
            reporter: None,
            status: Vec::new(),
            status_category: None,
            issue_type: Vec::new(),
            component: Vec::new(),
            label: Vec::new(),
            text: None,
            open: false,
            limit: 50,
            next_page_token: None,
            sort: "updated".to_string(),
            asc: false,
            desc: false,
        }
    }
}

#[derive(Debug, Args, Default)]
pub struct ShowArgs {
    pub issue_key: String,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub comments: bool,
}
