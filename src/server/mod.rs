use std::{fs::OpenOptions, os::fd::AsFd, path::PathBuf, rc::Rc};

use anyhow::{Result as AnyResult, bail};
use async_broadcast::{InactiveReceiver, Sender, broadcast};
use async_executor::LocalExecutor;
use bitcode::{decode, encode};
use derive_more::Constructor;
use futures::TryFutureExt;
use log::{debug, error, info};
use nix::{
    libc,
    unistd::{ForkResult, fork, setsid},
};
use smol::{
    Task,
    future::{self, FutureExt},
    io::{AsyncReadExt, AsyncWriteExt},
    net::unix::{UnixListener, UnixStream},
    stream::StreamExt,
};

use crate::{
    args::Args,
    common::{ClientMessage, ServerMessage},
};

thread_local! {
    static EX: LocalExecutor<'static> = const { LocalExecutor::new() };
}

fn spawn<T: 'static>(future: impl Future<Output = T> + 'static) -> Task<T> {
    EX.with(|ex| ex.spawn(future))
}

pub fn main(args: Args) -> AnyResult<()> {
    unsafe { daemonize()? };
    init_logger();
    let clean_up = CleanUp::new(&args);
    info!("Started");
    let reuslt = EX.with(|ex| {
        let result = future::block_on(ex.run(accept(args)));

        // make sure all jobs finished
        while ex.try_tick() {}
        result
    });
    clean_up.clean_up();
    info!("Exiting");
    reuslt
}

/// Hast to be called just once, right after forking from client
unsafe fn daemonize() -> AnyResult<()> {
    // Fork again to prevent terminal re-acquisition
    match unsafe { fork()? } {
        ForkResult::Parent { .. } => std::process::exit(0),
        ForkResult::Child => {}
    }

    // Detach from terminal
    setsid()?;

    // Change working directory
    std::env::set_current_dir("/")?;

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

#[derive(Debug)]
struct CleanUp {
    sock: PathBuf,
}

impl CleanUp {
    fn new(args: &Args) -> Self {
        Self {
            sock: args.socket.to_path_buf(),
        }
    }

    fn clean_up(self) {
        let _ = std::fs::remove_file(&self.sock);
    }
}

fn init_logger() {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/app.log")
        .unwrap();

    env_logger::Builder::new()
        .target(env_logger::Target::Pipe(Box::new(file)))
        .filter_level(log::LevelFilter::Debug)
        .init();
}

#[derive(Constructor, Debug)]
struct State {
    shutdown_tx: Sender<()>,
    shutdown_rx: InactiveReceiver<()>,
    message_tx: Sender<ClientMessage>,
    message_rx: InactiveReceiver<ClientMessage>,
}

async fn accept(args: Args) -> AnyResult<()> {
    let sock = UnixListener::bind(&args.socket)?;
    let mut incoming = sock.incoming();
    let (shutdown_tx, mut shutdown_rx) = broadcast(1);
    let (message_tx, message_rx) = broadcast(1024);
    let ctx = Rc::new(State::new(
        shutdown_tx,
        shutdown_rx.clone().deactivate(),
        message_tx,
        message_rx.deactivate(),
    ));

    let acceptor_fut = async move {
        while let Some(stream) = incoming.next().await {
            debug!("Received a connection");
            spawn(
                handle_client(stream?, ctx.clone())
                    .map_err(|e| error!("Error while handling client: {e}")),
            )
            .detach();
        }
        anyhow::Ok(())
    };

    let shutdown_fut = shutdown_rx.recv().map_err(anyhow::Error::from);

    acceptor_fut.or(shutdown_fut).await
}

async fn handle_client(mut stream: UnixStream, ctx: Rc<State>) -> AnyResult<()> {
    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await?;
    if n == 0 {
        debug!("Client sent an empty message");
        return Ok(());
    }
    let msg: ClientMessage = decode(&buf[..n])?;
    match msg {
        ClientMessage::Subscribe => {
            let mut rx = ctx.message_rx.clone().activate();
            let encoded = encode(&ServerMessage::Message {
                message: "HAIIII".to_string(),
            });
            stream.write_all(&encoded).await?;
            stream.flush().await?;
            while let Ok(message) = rx.recv().await {
                match message {
                    ClientMessage::Kill => {
                        let encoded = encode(&ServerMessage::Shutdown);
                        stream.write_all(&encoded).await?;
                        stream.flush().await?;
                    }
                    ClientMessage::Publish { message } => {
                        let encoded = encode(&ServerMessage::Message { message });
                        stream.write_all(&encoded).await?;
                        stream.flush().await?;
                    }
                    _ => bail!("Protocol error"),
                }
            }
        }
        ClientMessage::Kill => {
            let _ = ctx.shutdown_tx.try_broadcast(());
            let encoded = encode(&ServerMessage::Shutdown);
            stream.write_all(&encoded).await?;
        }
        msg @ ClientMessage::Publish { .. } => {
            let _ = ctx.message_tx.broadcast(msg).await;
        }
    }
    Ok(())
}
