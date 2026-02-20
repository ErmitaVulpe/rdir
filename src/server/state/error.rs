use bitcode::{Decode, Encode};
use derive_more::{Display, Error, From, IsVariant};

#[derive(Encode, Decode, Clone, Debug, Display, Error, PartialEq, Eq)]
#[display("Specified share doesnt exist")]
pub struct ShareDoesntExistError;

#[derive(Encode, Decode, Clone, Debug, Display, Error, PartialEq, Eq)]
#[display("Specified peer doesnt exist")]
pub struct PeerDoesntExistError;

#[derive(Encode, Decode, Clone, Debug, Display, Error, PartialEq, Eq)]
#[display("Specified peer already exists")]
pub struct RepeatedPeerError;

#[derive(Encode, Decode, Clone, Debug, Display, Error, PartialEq, Eq)]
#[display("Peer isnt connected to this share")]
pub struct PeerNotUsingShareError;

#[derive(Encode, Decode, Clone, Debug, Display, Error, PartialEq, Eq)]
#[display("Already connected to this share")]
pub struct RepeatedRemoteShareError;

#[derive(Encode, Decode, Clone, Debug, Display, Error, PartialEq, Eq)]
#[display("Specified remote share doesnt exist")]
pub struct NoSuchRemoteShareError;

#[derive(Encode, Decode, Clone, Debug, Display, Error, From, PartialEq, Eq, IsVariant)]
#[display("New peer failed to connect to a share")]
pub enum NewPeerConnectedToShareError {
    RepeatedPeer(RepeatedPeerError),
    ShareDoesntExist(ShareDoesntExistError),
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, From, PartialEq, Eq, IsVariant)]
#[display("Peer failed to connect to a share")]
pub enum PeerConnectedToShareError {
    PeerDoesntExist(PeerDoesntExistError),
    ShareDoesntExist(ShareDoesntExistError),
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, From, PartialEq, Eq, IsVariant)]
#[display("Couldnt disconnect peer from a share")]
pub enum PeerDisconnectedFromShareError {
    PeerNotUsingShare(PeerNotUsingShareError),
    ShareDoesntExist(ShareDoesntExistError),
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, From, PartialEq, Eq, IsVariant)]
#[display("Failed to kick a peer")]
pub enum KickPeerFromShareError {
    PeerNotUsingShare(PeerNotUsingShareError),
    ShareDoesntExist(ShareDoesntExistError),
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, PartialEq, Eq)]
#[display("Share with this name already exists")]
pub struct RepeatedShare;

#[derive(Encode, Decode, Clone, Debug, Display, Error, From, PartialEq, Eq, IsVariant)]
#[display("Failed to disconnect from a remote share")]
pub enum ExitPeerShareError {
    NoSuchConnectionError(NoSuchRemoteShareError),
}
