use std::path::PathBuf;

use clap::{Parser, Subcommand};
use derive_more::IsVariant;

#[derive(Parser, Debug)]
#[command(version, about)]
pub struct Args {
    #[command(subcommand)]
    pub command: Option<Command>,
    /// Specifies path to the unix socket for ipc
    #[arg(short, long, default_value = "/tmp/rdir.sock")]
    pub socket: PathBuf,
}

#[derive(Debug, IsVariant, Subcommand)]
pub enum Command {
    #[command(alias = "k")]
    Kill,
    #[command(alias = "m")]
    Message { message: String },
}
