use std::{fs::canonicalize, net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand, ValueHint};
use derive_more::IsVariant;
use tokio::io;

use crate::common::shares::{ShareLocator, ShareName};

#[derive(Parser, Debug)]
#[command(version, about)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
    /// Path to a dir for runtime files
    #[arg(
        env="RDIR_RUNDIR",
        global=true,
        hide_default_value=true,
        long="rundir",
        value_hint=ValueHint::DirPath,
    )]
    pub run_dir: Option<PathBuf>,
    /// Path to a dir for temp files
    #[arg(
        env="RDIR_STATEDIR",
        global=true,
        hide_default_value=true,
        long="statedir",
        value_hint=ValueHint::DirPath,
    )]
    pub state_dir: Option<PathBuf>,
    /// Server UDP bind socket
    #[arg(env = "RDIR_MAIN_SOCKET", global = true, long = "main-socket")]
    pub tcp_socket: Option<SocketAddr>,
}

impl Args {
    pub fn should_server_start(&self) -> bool {
        match &self.command {
            Command::Connect { command } => match command {
                ConnectCommand::Ls => false,
                ConnectCommand::Mount { .. } | ConnectCommand::Unmount { .. } => true,
            },
            Command::Discover => true,
            Command::Share { command } => match command {
                ShareCommand::Remove { .. } | ShareCommand::Share { .. } => true,
                ShareCommand::Ls => false,
            },
            Command::Kill | Command::Ls => false,
        }
    }

    fn add_dir_suffix(mut dir: PathBuf) -> PathBuf {
        dir.push("rdir");
        dir
    }

    pub fn get_run_dir(&self) -> io::Result<PathBuf> {
        match &self.run_dir {
            Some(path) => Some(path.to_owned()),
            None => dirs::runtime_dir().map(Self::add_dir_suffix),
        }
        .ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Parameter `rundir` unspecified and $XDG_RUNTIME_DIR env var unset",
        ))
    }

    pub fn get_state_dir(&self) -> io::Result<PathBuf> {
        match &self.state_dir {
            Some(path) => Some(path.to_owned()),
            None => dirs::state_dir().map(Self::add_dir_suffix),
        }
        .ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Parameter `statedir` unspecified and $XDG_STATE_HOME and $HOME env vars unset",
        ))
    }

    pub fn get_logs_dir(&self) -> io::Result<PathBuf> {
        let mut dir = self.get_state_dir()?;
        dir.push("logs");
        Ok(dir)
    }
}

#[derive(Debug, IsVariant, Subcommand)]
pub enum Command {
    /// manage remote shares
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
    /// List used remote shares
    #[command(short_flag = 'l', alias = "l")]
    Ls,
    /// Mount a new remote share
    #[command(short_flag = 'm', alias = "m")]
    Mount {
        /// Name of the remote share
        ///
        /// Specified as: ipv4/example
        /// or with port: ipv4:port/example
        #[arg()]
        name: ShareLocator,
        /// Path to a dir to mount the share
        #[arg(value_hint=ValueHint::DirPath, value_parser=existing_path_parser)]
        path: PathBuf,
    },
    /// Unmount a remote share
    #[command(short_flag = 'u', alias = "u")]
    Unmount {
        /// Name of the remote share, if ambiguous specify as <IP>:<NAME>
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
        name: ShareName,
    },
    /// create a new Share
    #[command(short_flag = 's', alias = "s")]
    Share {
        /// Path to dir a share
        #[arg(value_hint=ValueHint::DirPath, value_parser=existing_path_parser)]
        path: PathBuf,
        /// Name of the share, defaults to the name of the shared dir
        #[arg()]
        name: Option<ShareName>,
    },
}

fn existing_path_parser(s: &str) -> io::Result<PathBuf> {
    canonicalize(s)
}
