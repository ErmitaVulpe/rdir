use std::{path::Path, time::Duration};

use anyhow::{Context, Result as AnyResult, anyhow};
use backoff::{ExponentialBackoffBuilder, backoff::Backoff};
use bitcode::{decode, encode};
use smol::{
    LocalExecutor, Task, Timer, future,
    io::{self, AsyncReadExt, AsyncWriteExt},
    net::unix::UnixStream,
};

use crate::{
    args::{self, Args},
    common::{ClientMessage, ServerMessage},
};

thread_local! {
    static EX: LocalExecutor<'static> = const { LocalExecutor::new() };
}

fn spawn<T: 'static>(future: impl Future<Output = T> + 'static) -> Task<T> {
    EX.with(|ex| ex.spawn(future))
}

pub fn main(args: Args) -> AnyResult<()> {
    EX.with(|ex| {
        let result = future::block_on(ex.run(async_main(args)));

        // make sure all jobs finished
        while ex.try_tick() {}
        result
    })
}

async fn async_main(args: Args) -> AnyResult<()> {
    let mut stream = connect(&args).await.ok_or(anyhow!("No server running"))?;
    match args.command {
        None => loop {
            let buf = encode(&ClientMessage::Subscribe);
            stream
                .write(&buf)
                .await
                .context("Failed to send a subscribe message")?;
            let mut buf = vec![0u8; 1024];
            let n = stream.read(&mut buf).await?;

            let msg: ServerMessage = decode(&buf[..n]).context("Server sent an invalid message")?;
            println!("{:?}", msg);
        },
        Some(args::Command::Kill) => {
            let buf = encode(&ClientMessage::Kill);
            stream
                .write(&buf)
                .await
                .context("Failed to send a kill message")?;
            stream.flush().await?;
        }
        Some(args::Command::Message { message }) => {
            let buf = encode(&ClientMessage::Publish { message });
            stream
                .write_all(&buf)
                .await
                .context("Failed to send a message")?;
        }
    }

    Ok(())
}

async fn connect(args: &Args) -> Option<UnixStream> {
    // means the server was not just spawned, no need to retry
    if args.command.is_some() {
        return UnixStream::connect(&args.socket).await.ok();
    }

    let mut backoff = ExponentialBackoffBuilder::new()
        .with_initial_interval(Duration::from_millis(50))
        .with_randomization_factor(0.25)
        .with_max_interval(Duration::from_millis(250))
        .with_max_elapsed_time(Some(Duration::from_millis(1500)))
        .build();

    loop {
        match UnixStream::connect(&args.socket).await {
            Ok(val) => return Some(val),
            Err(_) => match backoff.next_backoff() {
                Some(delay) => {
                    Timer::after(delay).await;
                }
                None => return None,
            },
        }
    }
}
