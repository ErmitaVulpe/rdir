use std::{
    net::SocketAddrV4,
    sync::{Arc, LazyLock},
    time::Duration,
};

use derive_more::{Display, Error, From, IsVariant};
use libp2p::{Multiaddr, StreamProtocol, multiaddr, noise, tcp, yamux};
use libp2p_stream::Control;
use tokio::{
    net::UnixListener,
    select,
    signal::unix::{SignalKind, signal},
    spawn,
    sync::{RwLock, mpsc},
};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::{
    args::Args,
    common::shares::CommonShareNameParseError,
    server::{
        peer::accept_peers,
        state::{
            SharedState, State,
            error::{RepeatedShare, ShareDoesntExistError},
        },
        swarm::SwarmCommand,
    },
};

mod ipc;
mod peer;
mod setup;
pub mod state;
mod swarm;

pub const PROTOCOL_ADDR: StreamProtocol = StreamProtocol::new("/rdir/1.0");
pub const DOWNLOAD_CACHE_DIR: &str = "cache";
pub const LOGS_DIR: &str = "logs";
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
    runtime.block_on(main(&args, std_listener))?;
    runtime.shutdown_timeout(Duration::from_hours(69420));

    setup::clean_up(&args);
    Ok(())
}

async fn main(args: &Args, std_listener: std::os::unix::net::UnixListener) -> anyhow::Result<()> {
    // TEMP new_identity
    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::new(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(|_| libp2p_stream::Behaviour::new())?
        .build();

    let socket_addr = args
        .tcp_socket
        .unwrap_or(SocketAddrV4::new([0; 4].into(), NETWORK_PORT));
    swarm.listen_on(multiaddr_from_sock(&socket_addr))?;

    let state = Arc::new(RwLock::new(State::default()));
    let control = swarm.behaviour().new_control();
    let (swarm_command_tx, swarm_commamd_rx) = mpsc::channel(16);
    let server_ctx = ServerCtx {
        state,
        control,
        swarm_command_tx,
    };

    std_listener.set_nonblocking(true)?;
    let unix_listener = UnixListener::from_std(std_listener)?;
    let ipc_fut = ipc::accpet_client(unix_listener, server_ctx.clone());
    spawn(SERVER_CANCEL.run_until_cancelled(ipc_fut));
    spawn(SERVER_CANCEL.run_until_cancelled(accept_peers(server_ctx)));
    spawn(swarm::drive_swarm(swarm, swarm_commamd_rx));

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

fn multiaddr_from_sock(socket: &SocketAddrV4) -> Multiaddr {
    Multiaddr::from(*socket.ip()).with(multiaddr::Protocol::Tcp(socket.port()))
}

#[derive(Clone)]
struct ServerCtx {
    control: Control,
    state: SharedState,
    swarm_command_tx: mpsc::Sender<SwarmCommand>,
}

#[derive(Debug, Display, Error, From, IsVariant)]
#[display("Server encountered an error while processing the command")]
pub enum ServerError {
    #[display("Specified share name is invalid")]
    CommonShareNameParse(CommonShareNameParseError),
    // ConnectToRemoteShare(ConnectToRemoteShareError),
    InvalidShareName,
    // PeerIo(NoiseStreamError),
    RepeatedShare(RepeatedShare),
    ShareDoesntExit(ShareDoesntExistError),
    #[display("Path needs to be absolute")]
    RelativePath,
    #[display("Path needs to point to a directory")]
    PathNotDir,
}
