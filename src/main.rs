use std::{
    fs,
    os::unix::net::{UnixListener, UnixStream},
    path::Path,
};

use anyhow::{Context, Result as AnyResult};
use clap::Parser;
use nix::unistd::{ForkResult, fork};

use crate::server::SOCKET_NAME;

mod args;
mod client;
mod common;
mod server;

fn main() -> AnyResult<()> {
    let args = args::Args::parse();

    let sock_path = args.tmp_dir.join(SOCKET_NAME);
    let mut is_client = true;
    let maybe_sock = try_connect(&sock_path);
    let mut maybe_listener = None;
    if args.expects_active_server() && maybe_sock.is_none() {
        let _ = fs::create_dir(&args.tmp_dir);
        let listener = UnixListener::bind(&sock_path).context(format!(
            "Failed to create a unix socket at: {}",
            sock_path.to_string_lossy()
        ))?;

        match unsafe { fork() } {
            Ok(ForkResult::Parent { .. }) => {
                drop(listener);
            }
            Ok(ForkResult::Child) => {
                is_client = false;
                maybe_listener = Some(listener);
            }
            Err(e) => return Err(e).context("Failed to spawn the server"),
        }
    }

    match is_client {
        true => client::Client::run(args, maybe_sock),
        false => server::Server::run(args, maybe_listener.unwrap()),
    }
}

fn try_connect(sock_path: impl AsRef<Path>) -> Option<UnixStream> {
    let path = sock_path.as_ref();

    if path.exists() {
        let stream = UnixStream::connect(path);
        if stream.is_err() {
            let _ = fs::remove_file(path);
        }
        stream.ok()
    } else {
        None
    }
}
