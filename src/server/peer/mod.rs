use core::fmt;
use std::{net::SocketAddr, time::Duration};

use base58::ToBase58;
use bitcode::{Decode, Encode};
use blake2::{Blake2b, Digest, digest::consts::U32};
use derive_more::{Display, Error, From, IsVariant};
use tokio::{io, net::TcpStream, spawn, sync::mpsc};
use tracing::{error, info, warn};

use crate::{
    common::shares::ShareName,
    server::state::{
        PeerNotification, SharedState,
        error::{
            NewPeerConnectedToShareError, PeerConnectedToShareError, PeerDoesntExistError,
            RepeatedPeerError, ShareDoesntExistError,
        },
    },
};

pub mod call;

pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
pub const MESSAGE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeerId([u8; 32]);

impl From<&[u8]> for PeerId {
    fn from(value: &[u8]) -> Self {
        let mut hasher = Blake2b::<U32>::new();
        hasher.update(value);
        let res = hasher.finalize();
        Self(res.into())
    }
}

// impl From<&Keypair> for PeerId {
//     fn from(value: &Keypair) -> Self {
//         Self::from(value.public.as_slice())
//     }
// }

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let base58 = self.0.to_base58();
        write!(f, "{}", &base58[..f.width().unwrap_or(8)])
    }
}

pub async fn accept_peers(state: SharedState) {
    while let Some(incoming) = state.read().await.get_endpoint().accept().await {
        let state = state.clone();
        spawn(async move {
            let peer_addr = incoming.remote_address();
            let conn = match incoming.await {
                Ok(conn) => conn,
                Err(error) => {
                    info!(%peer_addr, %error, "Failed to accept a peer");
                    return;
                }
            };
        });
    }
    warn!("Main endpoint got closed prematurely. Daemon will stop accepting new peers");
}

async fn handle_peer(stream: TcpStream, peer_socket: SocketAddr, state: SharedState) {
    let result = handle_peer_inner(stream, peer_socket, state).await;
    if let Err(err) = result {
        error!("Error while handlig peer: {err}");
    }
}

async fn handle_peer_inner(
    stream: TcpStream,
    peer_socket: SocketAddr,
    state: SharedState,
) -> anyhow::Result<()> {
    // let SocketAddr::V4(peer_socket) = peer_socket else {
    //     bail!("Peer called from IPv6 which is unsupported");
    // };
    //
    // let noise = snowstorm::Builder::new(NOISE_PARAMS.clone())
    //     .local_private_key(&state.read().await.get_identity().private)
    //     .build_responder()?;
    //
    // // TODO add verifier for whitelist
    // let stream = NoiseStream::handshake(stream, noise)
    //     .timeout(HANDSHAKE_TIMEOUT)
    //     .await
    //     .context("Noise upgrade timed out")??;
    // info!(
    //     "Handshake successful, remote Peer Id: {}",
    //     PeerId::from(stream.get_state().get_remote_static().unwrap())
    // );
    // let peer_id: PeerId = stream.get_state().get_remote_static().unwrap().into();
    //
    // let mut conn = yamux::Connection::new(stream.compat(), Default::default(), yamux::Mode::Server);
    // let first_stream = poll_fn(|cx| conn.poll_next_inbound(cx))
    //     .timeout(MESSAGE_TIMEOUT)
    //     .await
    //     .context("Peer didnt open the first stream in time")?
    //     .context("Peer closed connection before first stream opened")??;
    //
    // debug!("AAAAAAAAAAAAAAAAA");
    //
    // let handle_stream_result =
    //     handle_stream_new_peer(first_stream, peer_id, peer_socket, state).await?;
    // match handle_stream_result {
    //     Some(rx) => {
    //         spawn(long_lived_conn_handler(conn, rx));
    //     }
    //     None => poll_fn(|cx| conn.poll_close(cx)).await?,
    // }

    Ok(())
}

/// returns whether this stream spawned a long-lived task
async fn handle_stream_new_peer(
    /* stream: yamux::Stream, */
    peer_id: PeerId,
    peer_socket: SocketAddr,
    state: SharedState,
) -> anyhow::Result<Option<mpsc::Receiver<PeerNotification>>> {
    todo!()

    // let mut stream = LengthDelimitedCodec::builder()
    //     .length_field_length(3)
    //     .new_framed(stream.compat());
    // let bytes = stream
    //     .next()
    //     .timeout(MESSAGE_TIMEOUT)
    //     .await?
    //     .context("Peer closed a stream before sending an init request")??;
    // let req: PeerInitReqDto = decode(&bytes)?;
    //
    // match req {
    //     PeerInitReqDto::JoinShare { name } => {
    //         let result =
    //             state
    //                 .write()
    //                 .await
    //                 .new_peer_connected_to_share(peer_id, peer_socket, name);
    //         match result {
    //             Ok(rx) => {
    //                 let bytes = encode(&PeerInitResp::Ok);
    //                 stream
    //                     .send(bytes.into())
    //                     .await
    //                     .context("Failed to send a response")?;
    //
    //                 spawn(long_lived_share_server(stream));
    //                 Ok(Some(rx))
    //             }
    //             Err(err) => {
    //                 let bytes = encode(&PeerInitResp::Err(err.into()));
    //                 stream
    //                     .send(bytes.into())
    //                     .await
    //                     .context("Failed to send a response")?;
    //                 Ok(None)
    //             }
    //         }
    //     }
    //     PeerInitReqDto::ListShares => {
    //         let names = state.read().await.get_share_names();
    //         let resp = PeerInitResp::ListShares { names };
    //         let bytes = encode(&resp);
    //         let res = stream.send(bytes.into()).await;
    //         match res {
    //             Ok(()) => Ok(None),
    //             Err(err) => Err(err).context("Failed to send a response"),
    //         }
    //     }
    // }
}

// async fn handle_stream(
//     stream: yamux::Stream,
//     peer_id: PeerId,
//     state: SharedState,
// ) -> anyhow::Result<PeerInitResp> {
//     let mut stream = LengthDelimitedCodec::builder()
//         .length_field_length(3)
//         .new_framed(stream.compat());
//     let bytes = stream
//         .next()
//         .timeout(MESSAGE_TIMEOUT)
//         .await?
//         .context("Peer closed a stream before sending an init request")??;
//     let req: PeerInitReqDto = decode(&bytes)?;
//
//     match req {
//         PeerInitReqDto::JoinShare { name } => {
//             let result = state.write().await.peer_connected_to_share(peer_id, name);
//             match result {
//                 Ok(()) => {
//                     let bytes = encode(&PeerInitResp::Ok);
//                     stream
//                         .send(bytes.into())
//                         .await
//                         .context("Failed to send a response")?;
//
//                     spawn(long_lived_share_server(stream));
//                     Ok()
//                 }
//                 Err(err) => {
//                     let bytes = encode(&PeerInitResp::Err(err.into()));
//                     stream
//                         .send(bytes.into())
//                         .await
//                         .context("Failed to send a response")?;
//                     Ok(())
//                 }
//             }
//         }
//         PeerInitReqDto::ListShares => {
//             let names = state.read().await.get_share_names();
//             let resp = PeerInitResp::ListShares { names };
//             let bytes = encode(&resp);
//             let res = stream.send(bytes.into()).await;
//             match res {
//                 Ok(()) => Ok(()),
//                 Err(err) => Err(err).context("Failed to send a response"),
//             }
//         }
//     }
// }

#[derive(Encode, Decode, Clone, Debug, IsVariant)]
enum PeerInitReqDto {
    /// Resp can be `PeerInitResp::{Err, Ok}`
    JoinShare { name: ShareName },
    /// Resp can be `PeerInitResp::ListShares`
    ListShares,
}

#[derive(Encode, Decode, Clone, Debug, IsVariant)]
pub enum PeerInitResp {
    Err(PeerInitError),
    ListShares { names: Vec<ShareName> },
    Ok,
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, From, IsVariant)]
pub enum PeerInitError {
    PeerDoesntExist(PeerDoesntExistError),
    RepeatedPeer(RepeatedPeerError),
    ShareDoesntExist(ShareDoesntExistError),
}

impl From<NewPeerConnectedToShareError> for PeerInitError {
    fn from(value: NewPeerConnectedToShareError) -> Self {
        match value {
            NewPeerConnectedToShareError::RepeatedPeer(err) => Self::RepeatedPeer(err),
            NewPeerConnectedToShareError::ShareDoesntExist(err) => Self::ShareDoesntExist(err),
        }
    }
}

impl From<PeerConnectedToShareError> for PeerInitError {
    fn from(value: PeerConnectedToShareError) -> Self {
        match value {
            PeerConnectedToShareError::PeerDoesntExist(err) => Self::PeerDoesntExist(err),
            PeerConnectedToShareError::ShareDoesntExist(err) => Self::ShareDoesntExist(err),
        }
    }
}

#[derive(Encode, Decode, Clone, Debug, Display, Error)]
#[display("Peer sent an invalid message for the current state")]
pub struct ProtocolError;

impl From<ProtocolError> for io::Error {
    fn from(value: ProtocolError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, value)
    }
}
