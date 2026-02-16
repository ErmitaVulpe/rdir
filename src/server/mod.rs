use std::{
    cell::RefCell,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    os::fd::AsFd,
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

use anyhow::{Context, Result as AnyResult, bail};
use async_broadcast::{InactiveReceiver, Sender, broadcast};
use bitcode::{decode, encode};
use derive_more::{Display, Error, From, IsVariant};
use futures::TryFutureExt;
use nix::{
    libc,
    unistd::{ForkResult, fork, setsid},
};
use smol::{
    LocalExecutor,
    channel::{Receiver, bounded, unbounded},
    future::FutureExt,
    net::{
        TcpListener, TcpStream,
        unix::{UnixListener, UnixStream},
    },
    stream::StreamExt,
};
use smol_timeout::TimeoutExt;
use tracing::{error, info, level_filters::LevelFilter};
use tracing_appender::non_blocking::WorkerGuard;

use crate::{
    args::Args,
    common::{
        ClientMessage, ConnectMessage, ServerError, ServerResponse, ShareMessage,
        framing::FramedStream,
        shares::{FullShareName, ShareName},
    },
    server::{
        messages::{PeerInitConnectToShareResponse, PeerInitListSharesRosponse, PeerInitMessage},
        net::{FramedError, FramedTcpStream},
        state::{Peer, PeerId, Share, ShareDoesntExistError, State, StateNotification},
    },
};

mod messages;
mod net;
pub mod state;

pub const DOWNLOAD_CACHE_DIR: &str = "cache";
pub const LOGS_DIR: &str = "logs";
pub const LOGS_PREFIX: &str = "rdir.log";
pub const SOCKET_NAME: &str = "rdir.sock";
/// 29284
pub const NETWORK_PORT: u16 = u16::from_be_bytes(*b"rd");

pub struct Server<'a> {
    ex: LocalExecutor<'a>,
    // TODO Check if want to hold on to this, maybe parse as config
    args: Args,
    state: RefCell<State>,
    shutdown_tx: Sender<()>,
    shutdown_rx: InactiveReceiver<()>,
}

impl Server<'_> {
    pub fn run(args: Args, std_listener: std::os::unix::net::UnixListener) -> AnyResult<()> {
        let _tracing_guard = Self::init(&args)?;
        info!("Init successful");
        let unix_listener: UnixListener = std_listener
            .try_into()
            .context("Failed to register the IPC socket as async")?;
        let tcp_listener: TcpListener = std::net::TcpListener::bind(
            args.tcp_socket
                .unwrap_or(SocketAddrV4::new(Ipv4Addr::LOCALHOST, NETWORK_PORT)),
        )?
        .try_into()?;

        let ex = LocalExecutor::new();
        let (shutdown_tx, mut shutdown_rx) = broadcast(1);
        let self_ = Rc::new(Self {
            ex,
            args,
            state: RefCell::new(State::default()),
            shutdown_tx,
            shutdown_rx: shutdown_rx.clone().deactivate(),
        });
        info!("Starting jobs");
        let client_fut = self_.clone().accept_client(unix_listener);
        let tcp_fut = self_.clone().accept_peer(tcp_listener);
        let main_fut = client_fut.or(tcp_fut);

        let result = smol::block_on(
            shutdown_rx
                .recv()
                .map_err(anyhow::Error::from)
                .or(self_.ex.run(main_fut)),
        );
        if let Err(ref err) = result {
            error!("{err}");
        }
        self_.clean_up();
        info!("Exitting");
        result
    }

    async fn accept_client(self: Rc<Self>, listener: UnixListener) -> AnyResult<()> {
        let mut incoming = listener.incoming();

        while let Some(stream) = incoming.next().await {
            let stream = stream?;
            self.ex.spawn(self.clone().handle_client(stream)).detach();
        }

        Ok(())
    }

    async fn handle_client(self: Rc<Self>, stream: UnixStream) {
        let value = async {
            let mut stream = FramedStream::new(stream);
            let buf = stream
                .read()
                .timeout(Duration::from_millis(500))
                .await
                .context("Timed out")??;
            let message: ClientMessage = decode(&buf)?;

            match message {
                ClientMessage::Connect(connect_message) => match connect_message {
                    ConnectMessage::Ls => {
                        let buf = encode(&self.state.borrow().remote_shares_dto());
                        stream.write(&buf).await?;
                    }
                    ConnectMessage::Mount { path, name } => {
                        let path = PathBuf::from(path);
                        match name {
                            ShareName::Common(_share_name) => todo!("Make autodiscovery"),
                            ShareName::Full(share_name) => {
                                self.connect_to_remote_share(share_name, path).await?
                            }
                        }
                    }
                    ConnectMessage::Unmount { name } => todo!(),
                },
                ClientMessage::Discover => todo!(),
                ClientMessage::Kill => {
                    let _ = self.shutdown_tx.try_broadcast(());
                    stream.write(&encode(&ServerResponse::Ok)).await?;
                }
                ClientMessage::Ls => {
                    let msg = {
                        let lock = self.state.borrow();
                        ServerResponse::Status {
                            peers: lock.peers_dto(),
                            remote_shares: lock.remote_shares_dto(),
                            shares: lock.shares_dto(),
                        }
                    };
                    let buf = encode(&msg);
                    stream.write(&buf).await?;
                }
                ClientMessage::Ping => {
                    stream.write(&encode(&ServerResponse::Pong)).await?;
                }
                ClientMessage::Share(share_message) => match share_message {
                    ShareMessage::Ls => {
                        let shares = self.state.borrow().shares_dto();
                        let msg = ServerResponse::LsShares(shares);
                        stream.write(&encode(&msg)).await?;
                    }
                    ShareMessage::Remove { name } => {
                        let msg: ServerResponse = self
                            .state
                            .borrow_mut()
                            .remove_share(&name, &self.shutdown_tx)
                            .into();
                        stream.write(&encode(&msg)).await?;
                    }
                    ShareMessage::Share { path, name } => {
                        let path = PathBuf::from(path);
                        let name = match name {
                            Some(val) => val,
                            None => {
                                let result = path
                                    .file_name()
                                    .ok_or(ServerError::InvalidShareName)
                                    .and_then(|n| n.to_string_lossy().parse().map_err(Into::into));
                                match result {
                                    Ok(val) => val,
                                    Err(err) => {
                                        stream
                                            .write(&encode(&ServerResponse::Err(err.clone())))
                                            .await?;
                                        return Err(anyhow::Error::new(err));
                                    }
                                }
                            }
                        };
                        let share = Share::new(name, path);
                        let msg: ServerResponse = self.state.borrow_mut().add_share(share).into();
                        stream.write(&encode(&msg)).await?;
                    }
                },
            }

            anyhow::Ok(())
        }
        .await;

        if let Err(err) = value {
            error!("Error during handling local client: {err}");
        }
    }

    async fn accept_peer(self: Rc<Self>, listener: TcpListener) -> AnyResult<()> {
        let mut incoming = listener.incoming();

        while let Some(stream) = incoming.next().await {
            let stream = stream?;
            self.ex.spawn(self.clone().handle_peer(stream)).detach();
        }

        Ok(())
    }

    async fn handle_peer(self: Rc<Self>, stream: TcpStream) {
        let value = async {
            let mut stream = FramedTcpStream::new_responder(stream).await?;
            let buf = stream
                .read()
                .timeout(Duration::from_millis(1000))
                .await
                .context("Timed out")??;
            let message: PeerInitMessage = decode(&buf)?;

            match message {
                PeerInitMessage::ConnectToShare { name } => {
                    let SocketAddr::V4(address) = stream.peer_addr()? else {
                        bail!("IPv6 is unsupported");
                    };
                    let (shutdown_tx, shutdown_rx) = bounded(1);
                    let (notification_tx, notification_rx) = unbounded();
                    let peer = Peer::new(address, shutdown_tx, notification_tx);
                    let result = self
                        .state
                        .borrow_mut()
                        .new_peer_connected_to_share(peer, name);
                    match result {
                        Ok(peer_id) => {
                            let buf = encode(&PeerInitConnectToShareResponse::Ok);
                            stream.write(&buf).await?;
                            self.long_lived_peer_connection(peer_id, shutdown_rx, notification_rx)
                                .await?;
                        }
                        Err(err) => {
                            let buf = encode(&PeerInitConnectToShareResponse::Err(err));
                            stream.write(&buf).await?;
                        }
                    }
                }
                PeerInitMessage::ListShares => {
                    let shares = self
                        .state
                        .borrow()
                        .get_shares()
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>();
                    let resp = PeerInitListSharesRosponse { shares };
                    let buf = encode(&resp);
                    stream.write(&buf).await?;
                }
            }

            anyhow::Ok(())
        }
        .await;

        if let Err(err) = value {
            error!("Error during handling TCP client: {err}");
        }
    }

    async fn connect_to_remote_share(
        self: Rc<Self>,
        share_name: FullShareName,
        mount_path: PathBuf,
    ) -> AnyResult<()> {
        let mut stream = FramedTcpStream::new_initiator((&share_name.addr).into()).await?;
        stream
            .write(&encode(&PeerInitMessage::ConnectToShare {
                name: share_name.name.clone(),
            }))
            .await?;
        let resp: PeerInitConnectToShareResponse =
            decode(&stream.read().await?).map_err(|_| ProtocolError)?;
        if let PeerInitConnectToShareResponse::Err(err) = resp {
            return Err(err.into());
        }

        let SocketAddr::V4(address) = stream.peer_addr()? else {
            bail!("IPv6 is unsupported");
        };
        let (shutdown_tx, shutdown_rx) = bounded(1);
        let (notification_tx, notification_rx) = unbounded();
        let peer = Peer::new(address, shutdown_tx, notification_tx);
        let peer_id = self
            .state
            .borrow_mut()
            .join_remote_share_new(peer, share_name, mount_path)?;
        let fut = self
            .clone()
            .long_lived_peer_connection(peer_id, shutdown_rx, notification_rx);
        self.ex.spawn(fut).detach();
        Ok(())
    }

    async fn list_peer_shares(
        self: Rc<Self>,
        addr: SocketAddrV4,
    ) -> Result<PeerInitListSharesRosponse, ListPeerSharesError> {
        let mut stream = FramedTcpStream::new_initiator(addr).await?;
        stream.write(&encode(&PeerInitMessage::ListShares)).await?;
        let resp: PeerInitListSharesRosponse =
            decode(&stream.read().await?).map_err(|_| ProtocolError)?;
        Ok(resp)
    }

    async fn long_lived_peer_connection(
        self: Rc<Self>,
        peer_id: PeerId,
        shutdown_rx: Receiver<()>,
        notification_rx: Receiver<StateNotification>,
    ) -> AnyResult<()> {
        info!("Entered the long living handler");
        smol::Timer::never().await;
        Ok(())
    }

    fn init(args: &Args) -> AnyResult<WorkerGuard> {
        unsafe {
            Self::daemonize(args)?;
        }
        let guard = Self::init_logs();
        let _ = std::fs::create_dir(DOWNLOAD_CACHE_DIR);
        Ok(guard)
    }

    fn init_logs() -> WorkerGuard {
        let file_appender = tracing_appender::rolling::daily(LOGS_DIR, LOGS_PREFIX);
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::fmt()
            .with_max_level(LevelFilter::DEBUG)
            .with_writer(non_blocking)
            .init();

        guard
    }

    unsafe fn daemonize(args: &Args) -> AnyResult<()> {
        // Fork again to prevent terminal re-acquisition
        match unsafe { fork()? } {
            ForkResult::Parent { .. } => std::process::exit(0),
            ForkResult::Child => {}
        }

        // Detach from terminal
        setsid()?;

        // Change working directory
        std::env::set_current_dir(&args.tmp_dir)?;

        // Reset file creation mask
        unsafe { libc::umask(0) };

        // Close standard fds
        for fd in 0..3 {
            unsafe { libc::close(fd) };
        }

        // Redirect stdin, stdout, stderr to /dev/null
        let devnull = std::fs::File::open("/dev/null")?;
        let devnull_fd = devnull.as_fd();
        let _ = nix::unistd::dup2_stdin(devnull_fd);
        let _ = nix::unistd::dup2_stdout(devnull_fd);
        let _ = nix::unistd::dup2_stderr(devnull_fd);

        Ok(())
    }

    fn clean_up(&self) {
        let _ = std::fs::remove_dir_all(".");
    }
}

#[derive(Clone, Debug, Display, Error)]
#[display("Other side sent an unexpected message")]
pub struct ProtocolError;

#[derive(Debug, Display, Error, From, IsVariant)]
#[display("Failed to list shares of a remote peer")]
pub enum ListPeerSharesError {
    Io(FramedError),
    ProtocolError(ProtocolError),
}

#[derive(Debug, Display, Error, From, IsVariant)]
#[display("Failed connect to a remote share")]
pub enum ConnectToRemoteShareError {
    Io(FramedError),
    ShareDoesntExist(ShareDoesntExistError),
    ProtocolError(ProtocolError),
}
