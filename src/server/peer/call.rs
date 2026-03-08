use std::{future::poll_fn, net::SocketAddrV4, path::PathBuf};

use bitcode::{decode, encode};
use derive_more::{Display, Error, From, IsVariant};
use futures::{SinkExt, StreamExt};
use snowstorm::NoiseStream;
use tokio::{io, net::TcpStream, select, spawn, sync::mpsc};
use tokio_util::{
    codec::{Framed, LengthDelimitedCodec},
    compat::{Compat, FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt},
    time::FutureExt,
};
use tracing::debug;

use crate::{
    common::shares::CommonShareName,
    server::{
        PeerId,
        peer::{
            HANDSHAKE_TIMEOUT, MESSAGE_TIMEOUT, NOISE_PARAMS, PeerInitResp, ProtocolError,
            conn::ConnDriver,
        },
        state::{PeerNotification, SharedState},
    },
};

pub async fn call_peer(
    peer_socket: SocketAddrV4,
    intent: PeerInitReq,
    state: SharedState,
) -> Result<PeerInitResp, CallPeerError> {
    let stream = TcpStream::connect(peer_socket)
        .timeout(HANDSHAKE_TIMEOUT)
        .await??;

    let noise = snowstorm::Builder::new(NOISE_PARAMS.clone())
        .local_private_key(&state.read().await.get_identity().private)
        .build_initiator()?;
    let stream = NoiseStream::handshake(stream, noise)
        .timeout(HANDSHAKE_TIMEOUT)
        .await??;
    let peer_id: PeerId = stream.get_state().get_remote_static().unwrap().into();
    debug!("Handshake successful, Peer Id: {peer_id}",);

    let mut conn = yamux::Connection::new(stream.compat(), Default::default(), yamux::Mode::Client);
    let first_stream = poll_fn(|cx| conn.poll_new_outbound(cx))
        .timeout(MESSAGE_TIMEOUT)
        .await??;
    debug!("Yamux upgrade successful");

    let (comman_tx, command_rx) = mpsc::channel(8);

    select! {
        _ = ConnDriver::new(
            conn,
            peer_id.clone(),
            command_rx,
            state.clone(),
        ) => {},
        _ = handle_stream_new_peer(first_stream, peer_id, peer_socket, intent, state) => {},
    };

    // let (spawned_stream_command_rx, resp) =
    //     handle_stream_new_peer(first_stream, peer_id, peer_socket, intent, state).await?;

    // match spawned_stream_command_rx {
    //     Some(rx) => {
    //         // spawn(long_lived_conn_handler(conn, rx));
    //         todo!()
    //     }
    //     None => poll_fn(|cx| conn.poll_close(cx)).await?,
    // }

    // Ok(resp)
    todo!()
}

#[derive(Debug, Display, Error, From, IsVariant)]
pub enum CallPeerError {
    Timeout(tokio::time::error::Elapsed),
    Io(io::Error),
    Noise(snowstorm::snow::Error),
    Snowstorm(snowstorm::SnowstormError),
    Yamux(yamux::ConnectionError),
}

/// returns whether this stream spawned a long-lived task
async fn handle_stream_new_peer(
    stream: yamux::Stream,
    peer_id: PeerId,
    peer_socket: SocketAddrV4,
    intent: PeerInitReq,
    state: SharedState,
) -> io::Result<(Option<mpsc::Receiver<PeerNotification>>, PeerInitResp)> {
    let mut stream = LengthDelimitedCodec::builder()
        .length_field_length(3)
        .new_framed(stream.compat());
    stream
        .send(encode(&intent.clone().into_dto()).into())
        .await?;
    debug!("Sent stream init request");

    let resp_bytes = stream
        .next()
        .timeout(MESSAGE_TIMEOUT)
        .await?
        .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))??;
    let resp: PeerInitResp =
        decode(&resp_bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    match intent {
        PeerInitReq::JoinShare { name, path } => match resp {
            PeerInitResp::Ok => {
                let result = state.write().await.new_peer_join_remote_share(
                    peer_id,
                    peer_socket,
                    name,
                    path,
                );
                match result {
                    Ok(rx) => {
                        spawn(long_lived_share_client(stream));
                        Ok((Some(rx), resp))
                    }
                    Err(_) => {
                        // if this branch is reached, it basically means that
                        // the peer lied to us
                        Err(ProtocolError.into())
                    }
                }
            }
            PeerInitResp::Err(_) => Ok((None, resp)),
            _ => Err(ProtocolError.into()),
        },
        PeerInitReq::ListShares => todo!(),
    }
}

pub async fn handle_stream(
    stream: yamux::Stream,
    peer_id: PeerId,
    intent: PeerInitReq,
    state: SharedState,
) -> io::Result<PeerInitResp> {
    let mut stream = LengthDelimitedCodec::builder()
        .length_field_length(3)
        .new_framed(stream.compat());
    stream
        .send(encode(&intent.clone().into_dto()).into())
        .await?;
    debug!("Sent stream init request");

    let resp_bytes = stream
        .next()
        .timeout(MESSAGE_TIMEOUT)
        .await?
        .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))??;
    let resp: PeerInitResp =
        decode(&resp_bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    match intent {
        PeerInitReq::JoinShare { name, path } => match resp {
            PeerInitResp::Ok => {
                let result = state.write().await.join_remote_share(peer_id, name, path);
                match result {
                    Ok(()) => {
                        spawn(long_lived_share_client(stream));
                        Ok(resp)
                    }
                    Err(_) => {
                        // if this branch is reached, it basically means that
                        // the peer lied to us
                        Err(ProtocolError.into())
                    }
                }
            }
            PeerInitResp::Err(_) => Ok(resp),
            _ => Err(ProtocolError.into()),
        },
        PeerInitReq::ListShares => todo!(),
    }
}

async fn long_lived_share_client(_stream: Framed<Compat<yamux::Stream>, LengthDelimitedCodec>) {
    debug!("Entered a long lived share client!");
}

#[derive(Clone, Debug, IsVariant)]
pub enum PeerInitReq {
    /// Resp can be `PeerInitResp::{Err, Ok}`
    JoinShare {
        name: CommonShareName,
        path: PathBuf,
    },
    /// Resp can be `PeerInitResp::ListShares`
    ListShares,
}

impl PeerInitReq {
    fn into_dto(self) -> super::PeerInitReqDto {
        match self {
            PeerInitReq::JoinShare { name, .. } => super::PeerInitReqDto::JoinShare { name },
            PeerInitReq::ListShares => super::PeerInitReqDto::ListShares,
        }
    }
}
