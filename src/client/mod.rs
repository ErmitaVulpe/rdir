use std::time::Duration;

use anyhow::{Context, Result as AnyResult};
use backoff::{ExponentialBackoffBuilder, backoff::Backoff};
use bitcode::{decode, encode};
use smol::{LocalExecutor, Timer, io, net::unix::UnixStream};

use crate::{
    args::Args,
    common::{ClientMessage, ServerMessage, framing::FramedStream},
    server::SOCKET_NAME,
};

pub struct Client<'a> {
    ex: LocalExecutor<'a>,
}

impl Client<'_> {
    pub fn run(args: Args, maybe_sock: Option<std::os::unix::net::UnixStream>) -> AnyResult<()> {
        let maybe_sock = maybe_sock
            .map(UnixStream::try_from)
            .transpose()
            .context("Failed to register the IPC socket as async")?;

        let ex = LocalExecutor::new();
        let self_ = Self { ex };

        smol::block_on(self_.ex.run(self_.main(args, maybe_sock)))
    }

    async fn main(&self, args: Args, maybe_sock: Option<UnixStream>) -> AnyResult<()> {
        let sock = match (maybe_sock, args.expects_active_server()) {
            (Some(val), _) => val,
            (None, false) => {
                println!("Server is down");
                return Ok(());
            }
            (None, true) => try_connect(&args).await.context(
                "Failed to connect to the newly spawned server. If this persists, there might be something wrong with the `tmpdir`. If it works on the second try, create a gh issue labeled \"I NEED MORE TIME\""
            )?,
        };
        let mut stream = FramedStream::new(sock);
        stream.write(&encode(&ClientMessage::from(&args))).await?;
        let resp: ServerMessage = decode(&stream.read().await?)?;
        println!("resp: {resp:?}");

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
                    Timer::after(delay).await;
                }
                None => return Err(e),
            },
        }
    }
}
