use std::{collections::BTreeMap, fmt, net::SocketAddr};

use bitcode::{Decode, Encode};
use derive_more::{From, IsVariant};

use crate::{
    args::{Args, ConnectCommand, ShareCommand},
    common::shares::{ShareLocator, ShareName},
    server::{
        PeerId, ServerError,
        state::{RemoteShare, Share},
    },
};

pub mod ipc;
pub mod shares;

#[derive(Encode, Decode, Clone, Debug, IsVariant)]
pub enum ClientMessage {
    Connect(ConnectMessage),
    Discover,
    Kill,
    Ls,
    Ping,
    Share(ShareMessage),
}

impl From<&Args> for ClientMessage {
    fn from(value: &Args) -> Self {
        match &value.command {
            crate::args::Command::Connect { command } => Self::Connect(command.into()),
            crate::args::Command::Discover => Self::Discover,
            crate::args::Command::Kill => Self::Kill,
            crate::args::Command::Ls => Self::Ls,
            crate::args::Command::Share { command } => Self::Share(command.into()),
        }
    }
}

#[derive(Encode, Decode, Clone, Debug, IsVariant)]
pub enum ConnectMessage {
    Ls,
    Mount { path: String, name: ShareLocator },
    Unmount { name: ShareName },
}

impl From<&ConnectCommand> for ConnectMessage {
    fn from(value: &ConnectCommand) -> Self {
        match &value {
            ConnectCommand::Ls => Self::Ls,
            ConnectCommand::Mount { name, path } => Self::Mount {
                path: path.to_string_lossy().to_string(),
                name: name.clone(),
            },
            ConnectCommand::Unmount { name } => Self::Unmount { name: name.clone() },
        }
    }
}

#[derive(Encode, Decode, Clone, Debug, IsVariant)]
pub enum ShareMessage {
    Ls,
    Remove {
        name: ShareName,
    },
    Share {
        path: String,
        name: Option<ShareName>,
    },
}

impl From<&ShareCommand> for ShareMessage {
    fn from(value: &ShareCommand) -> Self {
        match &value {
            ShareCommand::Ls => Self::Ls,
            ShareCommand::Remove { name } => Self::Remove { name: name.clone() },
            ShareCommand::Share { path, name } => Self::Share {
                path: path.to_string_lossy().to_string(),
                name: name.clone(),
            },
        }
    }
}

#[derive(Encode, Decode, Clone, Debug, From, IsVariant)]
pub enum ServerResponse {
    Err(ServerError),
    LsMountedShares(RemoteSharesDto),
    LsShares(BTreeMap<ShareName, Share>),
    Ok,
    Pong,
    Status {
        peers: PeersDto,
        remote_shares: RemoteSharesDto,
        shares: BTreeMap<ShareName, Share>,
    },
}

impl fmt::Display for ServerResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServerResponse::Err(err) => {
                writeln!(f, "error: {:?}", anyhow::Error::from(err.clone()))
            }
            ServerResponse::LsMountedShares(remote_shares_dto) => write!(f, "{remote_shares_dto}"),
            ServerResponse::LsShares(shares) => {
                writeln!(f, "Shares:")?;
                for (name, share) in shares {
                    writeln!(
                        f,
                        "{name} - {}, participants: {}",
                        share.path,
                        share.participants.len()
                    )?;
                }
                Ok(())
            }
            ServerResponse::Ok => Ok(()),
            ServerResponse::Pong => Ok(()),
            ServerResponse::Status {
                peers,
                remote_shares,
                shares,
            } => {
                writeln!(f, "{peers}")?;
                writeln!(f, "{remote_shares}")?;
                writeln!(f, "Shares:")?;
                for (name, share) in shares {
                    writeln!(
                        f,
                        "{name} - {}, participants: {}",
                        share.path,
                        share.participants.len()
                    )?;
                }
                Ok(())
            }
        }
    }
}

impl<E: Into<ServerError>> From<Result<(), E>> for ServerResponse {
    fn from(value: Result<(), E>) -> Self {
        match value {
            Ok(()) => Self::Ok,
            Err(err) => Self::Err(ServerError::from(err.into())),
        }
    }
}

#[derive(Encode, Decode, Clone, Debug)]
pub struct RemoteSharesDto(pub BTreeMap<PeerId, Vec<RemoteShareDto>>);

impl fmt::Display for RemoteSharesDto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Remote shares:")?;
        for (k, v) in &self.0 {
            writeln!(f, "  {k}")?;
            for remote_share in v {
                writeln!(f, "    {remote_share}")?;
            }
        }
        Ok(())
    }
}

#[derive(Encode, Decode, Clone, Debug)]
pub struct RemoteShareDto {
    pub name: ShareName,
    pub mount_path: String,
}

impl From<&RemoteShare> for RemoteShareDto {
    fn from(value: &RemoteShare) -> Self {
        Self {
            name: value.name.clone(),
            mount_path: value.mount_path.to_string_lossy().to_string(),
        }
    }
}

impl fmt::Display for RemoteShareDto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.mount_path)
    }
}

#[derive(Encode, Decode, Clone, Debug)]
pub struct PeersDto(pub BTreeMap<PeerId, SocketAddr>);

impl fmt::Display for PeersDto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Peers:")?;
        for (k, v) in &self.0 {
            // This will panic if OUR server sends bad data
            // But this shouldnt happen
            writeln!(f, "  {k}: {v}")?;
        }
        Ok(())
    }
}
