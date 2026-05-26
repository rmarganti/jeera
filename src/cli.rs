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
}

#[derive(Debug, Args)]
pub struct SearchArgs {}
