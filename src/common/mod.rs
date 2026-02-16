use std::{collections::BTreeMap, net::SocketAddrV4};

use bitcode::{Decode, Encode};
use derive_more::{Display, Error, From, IsVariant};

use crate::{
    args::{Args, ConnectCommand, ShareCommand},
    common::shares::{CommonShareName, CommonShareNameParseError, RemotePeerAddr, ShareName},
    server::state::{PeerId, RemoteShare, RepeatedShare, Share, ShareDoesntExistError},
};

pub mod framing;
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
    Mount { path: String, name: ShareName },
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
        name: CommonShareName,
    },
    Share {
        path: String,
        name: Option<CommonShareName>,
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
    LsMountedShares(BTreeMap<RemotePeerAddr, Vec<RemoteShareDto>>),
    LsShares(Vec<ShareDto>),
    Ok,
    Pong,
    Status {
        peers: BTreeMap<PeerId, SocketAddrV4>,
        remote_shares: BTreeMap<RemotePeerAddr, Vec<RemoteShareDto>>,
        shares: Vec<ShareDto>,
    },
}

impl<E: Into<ServerError>> From<Result<(), E>> for ServerResponse {
    fn from(value: Result<(), E>) -> Self {
        match value {
            Ok(()) => Self::Ok,
            Err(err) => Self::Err(err.into()),
        }
    }
}

#[derive(Encode, Decode, Clone, Debug)]
pub struct RemoteShareDto {
    pub name: CommonShareName,
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

#[derive(Encode, Decode, Clone, Debug)]
pub struct ShareDto {
    pub name: CommonShareName,
    pub path: String,
    pub participants: Vec<PeerId>,
}

impl From<&Share> for ShareDto {
    fn from(value: &Share) -> Self {
        Self {
            name: value.name.clone(),
            path: value.path.to_string_lossy().to_string(),
            participants: value.participants.iter().cloned().collect(),
        }
    }
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, From, IsVariant)]
#[display("Server encountered an error while processing the command")]
pub enum ServerError {
    #[display("Specified share name is invalid")]
    CommonShareNameParse(CommonShareNameParseError),
    InvalidShareName,
    RepeatedShare(RepeatedShare),
    ShareDoesntExit(ShareDoesntExistError),
}
