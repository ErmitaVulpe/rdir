use bitcode::{Decode, Encode};
use derive_more::IsVariant;

use crate::{common::shares::CommonShareName, server::state::NewPeerConnectedToShareError};

#[derive(Encode, Decode, Clone, Debug, IsVariant)]
pub enum PeerInitMessage {
    ConnectToShare { name: CommonShareName },
    ListShares,
}

#[derive(Encode, Decode, Clone, Debug, IsVariant)]
pub enum PeerInitConnectToShareResponse {
    Ok,
    Err(NewPeerConnectedToShareError),
}

#[derive(Encode, Decode, Clone, Debug)]
pub struct PeerInitListSharesRosponse {
    pub shares: Vec<CommonShareName>,
}

#[derive(Encode, Decode, Clone, Debug, IsVariant)]
pub enum PeerMessage {}

#[derive(Encode, Decode, Clone, Debug, IsVariant)]
pub enum PeerResponse {}
