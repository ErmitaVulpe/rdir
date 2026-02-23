use std::net::SocketAddrV4;

use bitcode::{Decode, Encode};
use derive_more::{Display, Error, From, IsVariant};
use futures::StreamExt;
use libp2p::swarm::DialError;
use tokio::sync::oneshot;

use crate::{
    common::shares::CommonShareName,
    server::{PROTOCOL_ADDR, ServerCtx, multiaddr_from_sock, swarm::SwarmCommand},
};

pub async fn accept_peers(mut server_ctx: ServerCtx) {
    let mut incoming = server_ctx.control.accept(PROTOCOL_ADDR).unwrap();
    while let Some((peer, stream)) = incoming.next().await {
        todo!();
    }
}

pub async fn call_peer(
    mut server_ctx: ServerCtx,
    addr: SocketAddrV4,
    intent: CallPeerIntent,
) -> Result<(), CallPeerError> {
    let addr = multiaddr_from_sock(&addr);
    let (resp_tx, resp_rx) = oneshot::channel();
    let swarm_command = SwarmCommand::Dial {
        addr,
        resp: resp_tx,
    };
    server_ctx
        .swarm_command_tx
        .send(swarm_command)
        .await
        .map_err(|_| CallPeerError::SwarmIsDead)?;
    let peer_id = resp_rx.await.map_err(|_| CallPeerError::SwarmIsDead)??;

    match intent {
        CallPeerIntent::ListShares => todo!(),
        CallPeerIntent::JoinShare { name } => todo!(),
    }

    todo!()
}

#[derive(Encode, Decode, Clone, Debug, IsVariant)]
pub enum CallPeerIntent {
    ListShares,
    JoinShare { name: CommonShareName },
}

#[derive(Debug, Display, Error, From, IsVariant)]
#[display("Failed to call a peer")]
pub enum CallPeerError {
    #[display("The swarm is dead")]
    SwarmIsDead,
    #[display("Failed to reach a peer at the pecified address")]
    SwarmDial(DialError),
}
