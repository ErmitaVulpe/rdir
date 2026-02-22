use std::{
    net::SocketAddrV4,
    sync::{Arc, LazyLock},
    time::Duration,
};

use derive_more::{Display, Error, From, IsVariant};
use futures::StreamExt;
use libp2p::{Multiaddr, StreamProtocol, multiaddr, noise, tcp, yamux};
use libp2p_stream::Control;
use tokio::{
    net::UnixListener,
    select,
    signal::unix::{SignalKind, signal},
    spawn,
    sync::RwLock,
};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::{args::Args, server::state::State};

mod ipc;
mod setup;
pub mod state;

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
    let state = Arc::new(RwLock::new(State::default()));

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
    let tcp_multiaddr =
        Multiaddr::from(*socket_addr.ip()).with(multiaddr::Protocol::Tcp(socket_addr.port()));
    swarm.listen_on(tcp_multiaddr)?;

    let control = swarm.behaviour().new_control();

    std_listener.set_nonblocking(true)?;
    let unix_listener = UnixListener::from_std(std_listener)?;
    let ipc_fut = ipc::accpet_client(unix_listener, state);
    spawn(SERVER_CANCEL.run_until_cancelled(ipc_fut));
    let peer_fut = accept_peers(control.clone());
    spawn(SERVER_CANCEL.run_until_cancelled(peer_fut));

    await_shutdown().await;
    Ok(())
}

async fn accept_peers(mut control: Control) {
    let mut incoming = control.accept(PROTOCOL_ADDR).unwrap();
    while let Some((peer, stream)) = incoming.next().await {
        todo!();
    }
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
    // #[display("Specified share name is invalid")]
    // CommonShareNameParse(CommonShareNameParseError),
    // ConnectToRemoteShare(ConnectToRemoteShareError),
    InvalidShareName,
    // PeerIo(NoiseStreamError),
    // RepeatedShare(RepeatedShare),
    // ShareDoesntExit(ShareDoesntExistError),
}
