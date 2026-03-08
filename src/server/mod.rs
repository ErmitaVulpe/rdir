use std::{
    net::SocketAddrV4,
    path::PathBuf,
    sync::{Arc, LazyLock},
    time::Duration,
};

use anyhow::Context;
use derive_more::{Display, Error, From, IsVariant};
use tokio::{
    net::{TcpListener, UnixListener},
    select,
    signal::unix::{SignalKind, signal},
    spawn,
    sync::RwLock,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::{
    args::Args,
    common::shares::CommonShareNameParseError,
    server::{
        peer::{NOISE_PARAMS, accept_peers, call::CallPeerError},
        state::{
            State,
            error::{ExitPeerShareError, RepeatedShare, ShareDoesntExistError},
        },
    },
};

mod ipc;
mod peer;
mod setup;
pub mod state;

pub use peer::PeerId;
pub use peer::ProtocolError as PeerProtocolError;

pub const DOWNLOAD_CACHE_DIR: &str = "cache";
pub static LOGS_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    let mut log_dir = dirs::state_dir().unwrap();
    log_dir.push("rdir");
    log_dir
});
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
    runtime.shutdown_timeout(Duration::from_hours(69420));

    setup::clean_up(&args);
    Ok(())
}

async fn main(args: &Args, std_listener: std::os::unix::net::UnixListener) -> anyhow::Result<()> {
    // TEMP new_identity
    let keypair = snowstorm::Builder::new(NOISE_PARAMS.clone()).generate_keypair()?;
    let local_peer_id = PeerId::from(&keypair);
    info!("Local peer id: {local_peer_id}");

    std_listener.set_nonblocking(true)?;
    let unix_listener =
        UnixListener::from_std(std_listener).context("Failed to bind the unix socket")?;

    let socket_addr = args
        .tcp_socket
        .unwrap_or(SocketAddrV4::new([0; 4].into(), NETWORK_PORT));
    let tcp_listener = TcpListener::bind(socket_addr)
        .await
        .context("Failed to bind the TCP socket")?;

    let state = Arc::new(RwLock::new(State::new(keypair)));

    let ipc_fut = ipc::accpet_client(unix_listener, state.clone());
    spawn(SERVER_CANCEL.run_until_cancelled(ipc_fut));
    spawn(SERVER_CANCEL.run_until_cancelled(accept_peers(tcp_listener, state.clone())));

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

#[derive(Debug, Display, Error, From, IsVariant)]
#[display("Server encountered an error while processing the command")]
pub enum ServerError {
    #[display("Failed to call a peer")]
    CallPeer(CallPeerError),
    #[display("Specified share name is invalid")]
    CommonShareNameParse(CommonShareNameParseError),
    ExitPeerShare(ExitPeerShareError),
    #[display("Path needs to point to a directory")]
    PathNotDir,
    Protocol(PeerProtocolError),
    #[display("Supplied share name is invalid")]
    RepeatedShare(RepeatedShare),
    #[display("Path needs to be absolute")]
    RelativePath,
    ShareDoesntExit(ShareDoesntExistError),
}
