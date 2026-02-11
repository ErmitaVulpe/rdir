use std::{os::fd::AsFd, rc::Rc, time::Duration};

use anyhow::{Context, Result as AnyResult};
use async_broadcast::{InactiveReceiver, Sender, broadcast};
use bitcode::{decode, encode};
use futures::TryFutureExt;
use nix::{
    libc,
    unistd::{ForkResult, fork, setsid},
};
use smol::{
    LocalExecutor,
    future::FutureExt,
    net::unix::{UnixListener, UnixStream},
    stream::StreamExt,
};
use smol_timeout::TimeoutExt;
use tracing::{debug, error, info, level_filters::LevelFilter};
use tracing_appender::non_blocking::WorkerGuard;

use crate::{
    args::Args,
    common::{ClientMessage, ServerMessage, framing::FramedStream},
};

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
    shutdown_tx: Sender<()>,
    shutdown_rx: InactiveReceiver<()>,
}

impl Server<'_> {
    pub fn run(args: Args, std_listener: std::os::unix::net::UnixListener) -> AnyResult<()> {
        let _tracing_guard = Self::init(&args)?;
        info!("Init successful");
        let listener: UnixListener = std_listener
            .try_into()
            .context("Failed to register the IPC socket as async")?;

        let ex = LocalExecutor::new();
        let (shutdown_tx, mut shutdown_rx) = broadcast(1);
        let self_ = Rc::new(Self {
            ex,
            args,
            shutdown_tx,
            shutdown_rx: shutdown_rx.clone().deactivate(),
        });
        info!("Starting jobs");
        let result = smol::block_on(
            shutdown_rx
                .recv()
                .map_err(anyhow::Error::from)
                .or(self_.ex.run(self_.clone().main(listener))),
        );
        if let Err(ref err) = result {
            error!("{err}");
        }
        self_.clean_up();
        info!("Exitting");
        result
    }

    async fn main(self: Rc<Self>, listener: UnixListener) -> AnyResult<()> {
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
            debug!("Client senf: {message:?}");
            stream.write(&encode(&ServerMessage::Pong)).await?;
            anyhow::Ok(())
        }
        .await;

        if let Err(err) = value {
            error!("Error during handling local client: {err}");
        }
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
