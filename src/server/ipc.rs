use std::path::PathBuf;

use tokio::{
    net::{UnixListener, UnixStream},
    spawn,
};
use tracing::{debug, error};

use crate::{
    common::{ClientMessage, ConnectMessage, ServerResponse, ShareMessage, ipc::IpcStream},
    server::{SERVER_CANCEL, ServerCtx, ServerError, state::Share},
};

pub async fn accpet_client(listener: UnixListener, server_ctx: ServerCtx) {
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                spawn(handle_client(stream, server_ctx.clone()));
            }
            Err(e) => error!("Error while accepting a local client: {e}"),
        }
    }
}

async fn handle_client(stream: UnixStream, ctx: ServerCtx) {
    let mut stream = IpcStream::new_server(stream);
    let message = match stream.read_command().await {
        Ok(val) => val,
        Err(err) => {
            error!("Failed to receive a command from local client {err}");
            return;
        }
    };
    debug!("Client sent: {message:?}");

    let result: Result<ServerResponse, ServerError> = async {
        match message {
            ClientMessage::Connect(message) => match message {
                ConnectMessage::Ls => {
                    let shares = ctx.state.read().await.remote_shares_dto();
                    Ok(ServerResponse::LsMountedShares(shares))
                }
                ConnectMessage::Mount { path, name } => todo!(),
                ConnectMessage::Unmount { name } => {
                    // let a = ctx.state.write().await.ex
                    todo!()
                }
            },
            ClientMessage::Discover => todo!(),
            ClientMessage::Kill => {
                SERVER_CANCEL.cancel();
                Ok(ServerResponse::Ok)
            }
            ClientMessage::Ls => {
                let lock = ctx.state.read().await;
                Ok(ServerResponse::Status {
                    peers: lock.peers_dto(),
                    remote_shares: lock.remote_shares_dto(),
                    shares: lock.shares_dto(),
                })
            }
            ClientMessage::Ping => Ok(ServerResponse::Ok),
            ClientMessage::Share(message) => match message {
                ShareMessage::Ls => {
                    let shares = ctx.state.read().await.shares_dto();
                    Ok(ServerResponse::LsShares(shares))
                }
                ShareMessage::Remove { name } => {
                    Ok(ctx.state.write().await.remove_share(&name).into())
                }
                ShareMessage::Share { path, name } => {
                    let path = PathBuf::from(path);
                    if path.is_relative() {
                        return Err(ServerError::RelativePath);
                    }
                    if !path.is_dir() {
                        return Err(ServerError::PathNotDir);
                    }
                    let name = name.unwrap_or(
                        path.file_name()
                            .ok_or(ServerError::PathNotDir)?
                            .to_string_lossy()
                            .to_string()
                            .parse()?,
                    );
                    let share = Share::new(name, path);
                    ctx.state.write().await.create_share(share)?;
                    Ok(ServerResponse::Ok)
                }
            },
        }
    }
    .await;

    let resp = result.unwrap_or_else(ServerResponse::from);
    if let ServerResponse::Err(e) = &resp {
        error!("Error while handling local client: {e}");
    }
    let _ = stream.write_respone(&resp).await;
}
