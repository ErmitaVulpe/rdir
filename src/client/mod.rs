use std::time::Duration;

use anyhow::{Context, Result as AnyResult};
use backoff::{ExponentialBackoffBuilder, backoff::Backoff};
use tokio::{io, net::UnixStream, time::sleep};

use crate::{
    args::Args,
    common::{ClientMessage, ipc::IpcStream},
    server::SOCKET_NAME,
};

pub struct Client;

impl Client {
    pub fn run(args: Args, maybe_sock: Option<std::os::unix::net::UnixStream>) -> AnyResult<()> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create an async runtime")
            .block_on(Self::main(args, maybe_sock))
    }

    async fn main(args: Args, maybe_sock: Option<std::os::unix::net::UnixStream>) -> AnyResult<()> {
        let stream = match (maybe_sock, args.expects_active_server()) {
            (Some(val), _) => {
                UnixStream::from_std(val).context("Failed to register the IPC socket as async")?
            },
            (None, false) => {
                println!("Server is down");
                return Ok(());
            }
            (None, true) => try_connect(&args).await.context(
                "Failed to connect to the newly spawned server. If this persists, there might be something wrong with the `tmpdir`. If it works on the second try however, create a gh issue labeled \"I NEED MORE TIME\""
            )?,
        };
        let mut ipc_stream = IpcStream::new_client(stream);
        let resp = ipc_stream.send_command(&ClientMessage::from(&args)).await?;
        println!("{resp}");
        Ok(())
    }
}

/// Tries to connect to the newly spawned server
async fn try_connect(args: &Args) -> io::Result<UnixStream> {
    let mut backoff = ExponentialBackoffBuilder::new()
        .with_initial_interval(Duration::from_millis(50))
        .with_randomization_factor(0.25)
        .with_max_interval(Duration::from_millis(250))
        .with_max_elapsed_time(Some(Duration::from_millis(1500)))
        .build();
    let sock = args.tmp_dir.join(SOCKET_NAME);

    loop {
        match UnixStream::connect(&sock).await {
            Ok(val) => return Ok(val),
            Err(e) => match backoff.next_backoff() {
                Some(delay) => {
                    sleep(delay).await;
                }
                None => return Err(e),
            },
        }
    }
}
