use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueHint};
use derive_more::IsVariant;

use crate::common::shares::{CommonShareName, ShareName};

#[derive(Parser, Debug)]
#[command(version, about)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
    /// Absolute path to a dir for temp files
    #[arg(
        default_value="/tmp",
        env="RDIR_TMPDIR",
        global=true,
        hide_default_value=true,
        long="tmpdir",
        short='t',
        value_hint=ValueHint::DirPath,
        value_parser=tmpdir_parser,
    )]
    pub tmp_dir: PathBuf,
}

impl Args {
    pub fn expects_active_server(&self) -> bool {
        match self.command {
            Command::Connect { .. } | Command::Discover | Command::Share { .. } => true,
            Command::Kill | Command::Ls => false,
        }
    }
}

#[derive(Debug, IsVariant, Subcommand)]
pub enum Command {
    /// manage Connections
    #[command(short_flag = 'C', alias = "c")]
    Connect {
        #[command(subcommand)]
        command: ConnectCommand,
    },
    /// Discover shares in the local network
    #[command(short_flag = 'D', alias = "d")]
    Discover,
    /// Kill the server, lets ongoing operations finish
    #[command(short_flag = 'K', alias = "k")]
    Kill,
    /// List shares and the status of the server
    #[command(short_flag = 'L', alias = "l")]
    Ls,
    /// manage Shares
    #[command(short_flag = 'S', alias = "s")]
    Share {
        #[command(subcommand)]
        command: ShareCommand,
    },
}

#[derive(Debug, IsVariant, Subcommand)]
pub enum ConnectCommand {
    /// List connections
    #[command(short_flag = 'l', alias = "l")]
    Ls,
    /// Mount a new connection
    #[command(short_flag = 'm', alias = "m")]
    Mount {
        /// Name of the remote share. If address is omitted, tries to search the local network
        #[arg()]
        name: ShareName,
        /// Path to a dir to mount the share
        #[arg(value_hint=ValueHint::DirPath)]
        path: PathBuf,
    },
    /// Unmount a connection
    #[command(short_flag = 'u', alias = "u")]
    Unmount {
        /// Name of the connection, if ambiguous specify as <IP>:<NAME>
        #[arg()]
        name: ShareName,
    },
}

#[derive(Debug, IsVariant, Subcommand)]
pub enum ShareCommand {
    /// List shares
    #[command(short_flag = 'l', alias = "l")]
    Ls,
    /// Remove a share
    #[command(short_flag = 'r', alias = "r")]
    Remove {
        /// Name of the share
        #[arg()]
        name: CommonShareName,
    },
    /// create a new Share
    #[command(short_flag = 's', alias = "s")]
    Share {
        /// Path to dir a share
        #[arg(value_hint=ValueHint::DirPath)]
        path: PathBuf,
        /// Name of the share, defaults to the name of the shared dir
        #[arg()]
        name: Option<CommonShareName>,
    },
}

fn tmpdir_parser(s: &str) -> Result<PathBuf, &'static str> {
    let mut path = PathBuf::from(s);
    if path.is_relative() {
        return Err("Value of tmpdir has to be an absolute path");
    }
    path.push("rdir");
    Ok(path)
}
