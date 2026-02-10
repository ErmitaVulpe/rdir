use std::{fs, os::unix::net::UnixStream, path::Path};

use anyhow::{Context, Result as AnyResult};
use clap::Parser;
use nix::unistd::{ForkResult, fork};

mod args;
mod client;
mod common;
mod server;

fn main() -> AnyResult<()> {
    let args = args::Args::parse();

    let mut is_client = true;
    let is_server_active = is_server_active(&args.socket);
    if args.command.is_none() && !is_server_active {
        match unsafe { fork() } {
            Ok(ForkResult::Parent { .. }) => {}
            Ok(ForkResult::Child) => {
                is_client = false;
            }
            Err(e) => return Err(e).context("Failed to spawn the server"),
        }
    }

    match is_client {
        true => client::main(args),
        false => server::main(args),
    }
}

/// Checks if the server is alive, if not, it cleans up an unused socket
fn is_server_active(sock: impl AsRef<Path>) -> bool {
    let path = sock.as_ref();
    if path.exists() {
        let b = UnixStream::connect(path).is_ok();
        if !b {
            let _ = fs::remove_file(path);
        }
        b
    } else {
        false
    }
}
