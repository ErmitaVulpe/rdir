use std::{collections::BTreeMap, fmt, net::SocketAddrV4};

use bitcode::{Decode, Encode};
use derive_more::{Display, Error, From, IsVariant};

use crate::{
    args::{Args, ConnectCommand, ShareCommand},
    common::shares::{CommonShareName, CommonShareNameParseError, RemotePeerAddr, ShareName},
    server::{
        ConnectToRemoteShareError, ProtocolError,
        net::FramedError,
        state::{
            PeerId, RemoteShare, RepeatedPeerError, RepeatedRemoteShareError, RepeatedShare, Share,
            ShareDoesntExistError,
        },
    },
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
    Err(ServerErrorDto),
    LsMountedShares(RemoteSharesDto),
    LsShares(SharesDto),
    Ok,
    Pong,
    Status {
        peers: PeersDto,
        remote_shares: RemoteSharesDto,
        shares: SharesDto,
    },
}

impl fmt::Display for ServerResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServerResponse::Err(err) => {
                writeln!(f, "error: {:?}", anyhow::Error::from(err.clone()))
            }
            ServerResponse::LsMountedShares(remote_shares_dto) => write!(f, "{remote_shares_dto}"),
            ServerResponse::LsShares(shares_dto) => write!(f, "{shares_dto}"),
            ServerResponse::Ok => Ok(()),
            ServerResponse::Pong => Ok(()),
            ServerResponse::Status {
                peers,
                remote_shares,
                shares,
            } => {
                writeln!(f, "{peers}")?;
                writeln!(f, "{remote_shares}")?;
                writeln!(f, "{shares}")
            }
        }
    }
}

impl<E: Into<ServerError>> From<Result<(), E>> for ServerResponse {
    fn from(value: Result<(), E>) -> Self {
        match value {
            Ok(()) => Self::Ok,
            Err(err) => Self::Err(ServerErrorDto::from(err.into())),
        }
    }
}

impl From<ServerError> for ServerResponse {
    fn from(value: ServerError) -> Self {
        Self::Err(value.into())
    }
}

#[derive(Encode, Decode, Clone, Debug)]
pub struct RemoteSharesDto(pub BTreeMap<RemotePeerAddr, Vec<RemoteShareDto>>);

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

impl fmt::Display for RemoteShareDto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.mount_path)
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

impl fmt::Display for ShareDto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  {}:", self.name)?;
        writeln!(f, "    path: {}", self.path)?;
        write!(
            f,
            "    participants: {}",
            self.participants
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )?;
        for peer in &self.participants {
            write!(f, "{peer} ")?;
        }
        Ok(())
    }
}

#[derive(Encode, Decode, Clone, Debug)]
pub struct PeersDto(pub BTreeMap<PeerId, SocketAddrV4>);

impl fmt::Display for PeersDto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Peers:")?;
        for (k, v) in &self.0 {
            writeln!(f, "  {k}: {v}")?;
        }
        Ok(())
    }
}

#[derive(Encode, Decode, Clone, Debug)]
pub struct SharesDto(pub Vec<ShareDto>);

impl fmt::Display for SharesDto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Shares:")?;
        for share in &self.0 {
            writeln!(f, "{share}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Display, Error, From, IsVariant)]
#[display("Server encountered an error while processing the command")]
pub enum ServerError {
    #[display("Specified share name is invalid")]
    CommonShareNameParse(CommonShareNameParseError),
    ConnectToRemoteShare(ConnectToRemoteShareError),
    InvalidShareName,
    PeerIo(FramedError),
    RepeatedShare(RepeatedShare),
    ShareDoesntExit(ShareDoesntExistError),
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, From, IsVariant)]
pub enum ServerErrorDto {
    #[display("Specified share name is invalid")]
    CommonShareNameParse(CommonShareNameParseError),
    ConnectToRemoteShare(ConnectToRemoteShareErrorDto),
    InvalidShareName,
    #[display("Error while communicating with a peer")]
    PeerIo(FramedErrorDto),
    RepeatedShare(#[error(ignore)] RepeatedShare),
    ShareDoesntExit(#[error(ignore)] ShareDoesntExistError),
}

impl From<ServerError> for ServerErrorDto {
    fn from(value: ServerError) -> Self {
        match value {
            ServerError::CommonShareNameParse(err) => Self::CommonShareNameParse(err),
            ServerError::ConnectToRemoteShare(err) => Self::ConnectToRemoteShare(err.into()),
            ServerError::InvalidShareName => todo!(),
            ServerError::PeerIo(err) => Self::PeerIo(err.into()),
            ServerError::RepeatedShare(err) => Self::RepeatedShare(err),
            ServerError::ShareDoesntExit(err) => Self::ShareDoesntExit(err),
        }
    }
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, IsVariant)]
#[display("Error with Encrypted IO")]
pub enum FramedErrorDto {
    #[display("{_0}")]
    Crypto(#[error(ignore)] String),
    #[display("{_0}")]
    Io(#[error(ignore)] String),
}

impl From<FramedError> for FramedErrorDto {
    fn from(value: FramedError) -> Self {
        match value {
            FramedError::Io(err) => Self::Io(anyhow::Error::from(err).to_string()),
            FramedError::Crypto(err) => Self::Crypto(anyhow::Error::from(err).to_string()),
        }
    }
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, IsVariant)]
pub enum ConnectToRemoteShareErrorDto {
    #[display("{_0}")]
    Io(#[error(ignore)] String),
    ShareDoesntExist(ShareDoesntExistError),
    RepeatedRemoteShare(RepeatedRemoteShareError),
    RepeatedPeer(RepeatedPeerError),
    ProtocolError(ProtocolError),
}

impl From<ConnectToRemoteShareError> for ConnectToRemoteShareErrorDto {
    fn from(value: ConnectToRemoteShareError) -> Self {
        match value {
            ConnectToRemoteShareError::Io(err) => Self::Io(anyhow::Error::from(err).to_string()),
            ConnectToRemoteShareError::ShareDoesntExist(err) => Self::ShareDoesntExist(err),
            ConnectToRemoteShareError::RepeatedRemoteShare(err) => Self::RepeatedRemoteShare(err),
            ConnectToRemoteShareError::RepeatedPeer(err) => Self::RepeatedPeer(err),
            ConnectToRemoteShareError::ProtocolError(err) => Self::ProtocolError(err),
        }
    }
}
