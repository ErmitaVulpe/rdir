use bitcode::{Decode, Encode};
use derive_more::IsVariant;

use crate::{
    args::{Args, ConnectCommand, ShareCommand},
    common::shares::{CommonShareName, ShareName},
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
    Mount {
        path: String,
        name: ShareName,
    },
    Unmount {
        name: ShareName,
    },
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
    Remove { name: CommonShareName },
    Share { path: String, name: Option<CommonShareName> },
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

#[derive(Encode, Decode, Clone, Debug, IsVariant)]
pub enum ServerMessage {
    Pong,
}
