use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "jeera")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Search(SearchArgs),
    Show(ShowArgs),
}

#[derive(Debug, Args, Default)]
pub struct SearchArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args, Default)]
pub struct ShowArgs {
    pub issue_key: String,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub comments: bool,
}
