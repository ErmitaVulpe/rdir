use std::{
    net::SocketAddr,
    sync::{Arc, LazyLock},
};

use anyhow::Context;
use bitcode::{Decode, Encode};
use derive_more::{Display, Error, From, IsVariant};
use quinn::crypto::rustls::QuicServerConfig;
use tokio::{
    net::UnixListener,
    select,
    signal::unix::{SignalKind, signal},
    spawn,
    sync::RwLock,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::{
    args::Args,
    common::shares::ValueTooLong,
    server::{
        peer::call::CallPeerError,
        state::{State, StateError},
    },
};

mod identity;
mod ipc;
mod peer;
mod setup;
pub mod state;

pub use peer::PeerId;

pub const DOWNLOAD_CACHE_DIR: &str = "cache";
pub const LOGS_PREFIX: &str = "rdir.log";
pub const SOCKET_NAME: &str = "rdir.sock";
/// 29284
pub const NETWORK_PORT: u16 = u16::from_be_bytes(*b"rd");

static SERVER_CANCEL: LazyLock<CancellationToken> = LazyLock::new(Default::default);

pub fn run(args: Args, std_listener: std::os::unix::net::UnixListener) -> anyhow::Result<()> {
    let _guard = unsafe { setup::setup(&args) }?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        if let Err(err) = main(&args, std_listener).await {
            error!("{:#}", err.context("Error while initializing the server"));
        }
    });
    drop(runtime);

    setup::clean_up(&args);
    Ok(())
}

async fn main(args: &Args, std_listener: std::os::unix::net::UnixListener) -> anyhow::Result<()> {
    let identity = identity::Identity::load_or_new(args.get_state_dir()?)?;

    std_listener.set_nonblocking(true)?;
    let unix_listener =
        UnixListener::from_std(std_listener).context("Failed to bind the unix socket")?;

    let socket_addr = args
        .tcp_socket
        .unwrap_or(SocketAddr::new([0; 4].into(), NETWORK_PORT));

    let (server_config, peer_id) = identity.into_server_config()?;
    let server_config = quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(
        Arc::new(server_config),
    )?));
    let endpoint = quinn::Endpoint::server(server_config, socket_addr)?;

    let state = Arc::new(RwLock::new(State::new(endpoint)));

    info!("Starting server with local peer id: {peer_id}");

    spawn(SERVER_CANCEL.run_until_cancelled(ipc::accpet_client(unix_listener, state.clone())));
    spawn(SERVER_CANCEL.run_until_cancelled(peer::accept_peers(state)));

    await_shutdown().await;
    Ok(())
}

async fn await_shutdown() {
    let mut sigint = signal(SignalKind::interrupt()).unwrap();
    let mut sigterm = signal(SignalKind::terminate()).unwrap();
    let signals_fut = async {
        select! {
            _ = sigint.recv() => {},
            _ = sigterm.recv() => {},
        }
    };

    select! {
        _ = signals_fut => {
            SERVER_CANCEL.cancel();
            info!("Received a shutdown signal");
        },
        _ = SERVER_CANCEL.cancelled() => {}
    }

    info!("Shutting down");
}

#[derive(Encode, Decode, Clone, Debug, Display, Error, From, IsVariant)]
#[display("Server encountered an error while processing the command")]
pub enum ServerError {
    StateError(StateError),
    #[display("Failed to call a peer")]
    CallPeer(CallPeerError),
    #[display("Specified share name is invalid")]
    CommonShareNameParse(ValueTooLong),
    #[display("Path needs to point to a directory")]
    PathNotDir,
    #[display("Path needs to be absolute")]
    RelativePath,
}
