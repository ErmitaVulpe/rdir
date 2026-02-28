use core::fmt;
use std::{
    net::{SocketAddr, SocketAddrV4},
    sync::LazyLock,
    task::ready,
    time::Duration,
};

use anyhow::{Context, bail};
use base58::ToBase58;
use bitcode::{Decode, Encode, decode, encode};
use blake2::{Blake2b, Digest, digest::consts::U32};
use derive_more::{Display, Error, From, IsVariant};
use futures::{SinkExt, StreamExt, future::poll_fn};
use snowstorm::{Keypair, NoiseParams, NoiseStream};
use tokio::{
    net::{TcpListener, TcpStream},
    spawn,
};
use tokio_util::{
    codec::{Framed, LengthDelimitedCodec},
    compat::{Compat, FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt},
    time::FutureExt,
};
use tracing::{debug, error, info};

use crate::{
    common::shares::CommonShareName,
    server::state::{Peer, SharedState, error::NewPeerConnectedToShareError},
};

pub mod call;

pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
pub const MESSAGE_TIMEOUT: Duration = Duration::from_secs(2);

pub static NOISE_PARAMS: LazyLock<NoiseParams> =
    LazyLock::new(|| "Noise_XX_25519_AESGCM_BLAKE2b".parse().unwrap());

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

impl From<&Keypair> for PeerId {
    fn from(value: &Keypair) -> Self {
        Self::from(value.public.as_slice())
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let base58 = self.0.to_base58();
        write!(f, "{}", &base58[..f.width().unwrap_or(8)])
    }
}

pub async fn accept_peers(tcp_listener: TcpListener, state: SharedState) {
    while let Ok((stream, peer_socket)) = tcp_listener.accept().await {
        spawn(handle_peer(stream, peer_socket, state.clone()));
    }
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
    let SocketAddr::V4(peer_socket) = peer_socket else {
        bail!("Peer called from IPv6 which is unsupported");
    };

    let noise = snowstorm::Builder::new(NOISE_PARAMS.clone())
        .local_private_key(&state.read().await.get_identity().private)
        .build_responder()?;

    // TODO add verifier
    let stream = NoiseStream::handshake(stream, noise)
        .timeout(HANDSHAKE_TIMEOUT)
        .await
        .context("Noise upgrade timed out")??;
    info!(
        "Handshake successful, Peer Id: {}",
        PeerId::from(stream.get_state().get_remote_static().unwrap())
    );
    let peer_id: PeerId = stream.get_state().get_remote_static().unwrap().into();

    let mut conn = yamux::Connection::new(stream.compat(), Default::default(), yamux::Mode::Server);
    let first_stream = poll_fn(|cx| conn.poll_next_inbound(cx))
        .await
        .context("Peer closed connection before first stream opened")??;

    let keep_alive = handle_stream(first_stream, peer_id, peer_socket, state).await?;
    match keep_alive {
        true => {
            spawn(long_lived_conn_handler(conn));
        }
        false => poll_fn(|cx| conn.poll_close(cx)).await?,
    }

    Ok(())
}

async fn long_lived_conn_handler(mut conn: yamux::Connection<Compat<NoiseStream<TcpStream>>>) {
    let _: () = poll_fn(|cx| {
        let _stream = ready!(conn.poll_next_inbound(cx));
        unimplemented!()
    })
    .await;
}

/// returns whether this stream spawned a long-lived task
async fn handle_stream(
    stream: yamux::Stream,
    peer_id: PeerId,
    peer_socket: SocketAddrV4,
    state: SharedState,
) -> anyhow::Result<bool> {
    let mut stream = LengthDelimitedCodec::builder()
        .length_field_length(3)
        .new_framed(stream.compat());
    let bytes = stream
        .next()
        .timeout(MESSAGE_TIMEOUT)
        .await?
        .context("Peer closed a stream before sending an init request")??;
    let req: PeerInitReq = decode(&bytes)?;

    match req {
        PeerInitReq::JoinShare { name } => {
            let peer = Peer::new(peer_socket);
            let result = state
                .write()
                .await
                .new_peer_connected_to_share(peer_id, peer, name);
            let resp = match result {
                Ok(()) => PeerInitResp::Ok,
                Err(err) => PeerInitResp::Err(err.into()),
            };
            let bytes = encode(&resp);
            let res = stream.send(bytes.into()).await;
            match res {
                Ok(()) => {
                    spawn(long_lived_share_server(stream));
                    Ok(true)
                }
                Err(err) => Err(err).context("Failed to send a response"),
            }
        }
        PeerInitReq::ListShares => {
            let names = state.read().await.get_share_names();
            let resp = PeerInitResp::ListShares { names };
            let bytes = encode(&resp);
            let res = stream.send(bytes.into()).await;
            match res {
                Ok(()) => Ok(false),
                Err(err) => Err(err).context("Failed to send a response"),
            }
        }
    }
}

async fn long_lived_share_server(_stream: Framed<Compat<yamux::Stream>, LengthDelimitedCodec>) {
    debug!("Entered a long lived peer handler!");
}

#[derive(Encode, Decode, Clone, Debug)]
enum PeerInitReq {
    JoinShare { name: CommonShareName },
    ListShares,
}

#[derive(Encode, Decode, Clone, Debug)]
enum PeerInitResp {
    Err(PeerInitError),
    ListShares { names: Vec<CommonShareName> },
    Ok,
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, From, IsVariant)]
enum PeerInitError {
    NewPeerConnectedToShare(NewPeerConnectedToShareError),
}
