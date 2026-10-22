use core::fmt;
use std::{str::FromStr, time::Duration};

use anyhow::Context;
use bitcode::{Decode, Encode};
use blake2::{Blake2b, Digest, digest::consts::U32};
use derive_more::{Display, Error};
use quinn::{Connection, RecvStream, SendStream, rustls::pki_types::CertificateDer};
use tokio::spawn;
use tokio_util::{sync::CancellationToken, time::FutureExt};
use tracing::{error, info, warn};

use crate::server::{
    identity::peer_id_from_certder,
    state::{Peer, SharedState},
};

pub mod call;

pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeerId([u8; 32]);

impl PeerId {
    pub fn from_raw_pubkey(value: &[u8]) -> Self {
        let mut hasher = Blake2b::<U32>::new();
        hasher.update(value);
        let res = hasher.finalize();
        Self(res.into())
    }
}

impl FromStr for PeerId {
    type Err = PeerIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let raw = bs58::decode(s).into_vec().map_err(|_| PeerIdParseError)?;
        Ok(Self(raw.try_into().map_err(|_| PeerIdParseError)?))
    }
}

#[derive(Display, Debug, Error)]
#[display("Invalid peer ID")]
pub struct PeerIdParseError;

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let base58 = bs58::encode(&self.0).into_string();
        match f.precision() {
            Some(precision) => write!(f, "{base58:.precision$}"),
            None => f.write_str(&base58),
        }
    }
}

pub async fn accept_peers(state: SharedState) {
    let endpoint = {
        let state = state.read().await;
        state.get_endpoint().clone()
    };
    while let Some(incoming) = endpoint.accept().await {
        let state = state.clone();
        spawn(async move {
            let conn = match incoming.into_future().timeout(HANDSHAKE_TIMEOUT).await {
                Ok(Ok(conn)) => conn,
                Ok(Err(error)) => {
                    info!(%error, "Failed to accept a peer");
                    return;
                }
                Err(error) => {
                    info!("{error}");
                    return;
                }
            };

            let result = (|| {
                let peer_identity = conn.peer_identity().context("Peer gave no identity")?;
                let peer_cert: Box<Vec<CertificateDer>> = peer_identity
                    .downcast()
                    .map_err(|_| anyhow::anyhow!("Failed to downcast peer identity"))?;
                peer_id_from_certder(peer_cert.get(0).context("Peer gave no certificate")?)
                    .context("Failed to get peer id from peer cert")
            })();

            let peer_id = match result {
                Ok(peer_id) => peer_id,
                Err(err) => {
                    info!(%err, "Failed to accept a peer");
                    return;
                }
            };

            let cancel_token = {
                let mut lock = state.write().await;
                let peer = Peer::new(conn.clone(), peer_id.clone());
                let cancel_token = peer.cancel_token.clone();
                lock.new_peer_connection(peer).unwrap();
                cancel_token
            };

            accept_peer_channels(conn, peer_id, state, cancel_token).await;
        });
    }
    warn!("Main endpoint got closed prematurely. Daemon will stop accepting new peers");
}

async fn accept_peer_channels(
    conn: Connection,
    peer_id: PeerId,
    state: SharedState,
    cancel_token: CancellationToken,
) {
    cancel_token
        .run_until_cancelled(async {
            loop {
                match conn.accept_bi().await {
                    Ok(channel) => {
                        spawn(handle_peer_channel(channel, peer_id.clone(), state.clone()));
                    }
                    Err(err) => {
                        error!(%err, "Error while accepting a peer channel");
                        cancel_token.cancel();
                        break;
                    }
                }
            }
        })
        .await;
}

async fn handle_peer_channel(
    channel: (SendStream, RecvStream),
    peer_id: PeerId,
    state: SharedState,
) {
    info!("ACCEPTED PEER CHANNEL :D");
}
