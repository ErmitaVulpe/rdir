use std::{fs::canonicalize, net::SocketAddrV4, path::PathBuf};

use clap::{Parser, Subcommand, ValueHint};
use derive_more::IsVariant;
use tokio::io;

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
    /// Server TCP bind socket
    #[arg(env = "RDIR_TCP_SOCKET", global = true, long = "tcp-socket")]
    pub tcp_socket: Option<SocketAddrV4>,
    /// Server UDP bind socket
    #[arg(env = "RDIR_UDP_SOCKET", global = true, long = "udp-socket")]
    pub udp_socket: Option<SocketAddrV4>,
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
        /// Name of the remote share. If address is omitted, tries to search the local network
        #[arg()]
        name: ShareName,
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
        name: CommonShareName,
    },
    /// create a new Share
    #[command(short_flag = 's', alias = "s")]
    Share {
        /// Path to dir a share
        #[arg(value_hint=ValueHint::DirPath, value_parser=existing_path_parser)]
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
    path.push(format!("rdir-{}", nix::unistd::Uid::current()));
    Ok(path)
}

fn existing_path_parser(s: &str) -> io::Result<PathBuf> {
    canonicalize(s)
}
