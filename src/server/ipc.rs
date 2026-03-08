use std::path::PathBuf;

use tokio::{
    net::{UnixListener, UnixStream},
    spawn,
};
use tracing::{debug, error, info};

use crate::{
    common::{
        ClientMessage, ConnectMessage, ServerResponse, ShareMessage, ipc::IpcStream,
        shares::FullShareName,
    },
    server::{
        SERVER_CANCEL, ServerError,
        peer::call::PeerInitReq,
        state::{Share, SharedState},
    },
};

pub async fn accpet_client(listener: UnixListener, state: SharedState) {
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let state_clone = state.clone();
                spawn(async move {
                    handle_client(stream, &state_clone).await;
                    state_clone.read().await.try_close_server();
                });
            }
            Err(e) => error!("Error while accepting a local client: {e}"),
        }
    }
}

async fn handle_client(stream: UnixStream, state: &SharedState) {
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
                    let shares = state.read().await.remote_shares_dto();
                    Ok(ServerResponse::LsMountedShares(shares))
                }
                ConnectMessage::Mount { path, name } => {
                    mount_share(validate_path(path)?, name, state.clone()).await
                }
                ConnectMessage::Unmount { name } => {
                    Ok(state.write().await.exit_remote_share(name).into())
                }
            },
            ClientMessage::Discover => todo!(),
            ClientMessage::Kill => {
                SERVER_CANCEL.cancel();
                Ok(ServerResponse::Ok)
            }
            ClientMessage::Ls => {
                let lock = state.read().await;
                Ok(ServerResponse::Status {
                    peers: lock.peers_dto(),
                    remote_shares: lock.remote_shares_dto(),
                    shares: lock.shares_dto(),
                })
            }
            ClientMessage::Ping => Ok(ServerResponse::Ok),
            ClientMessage::Share(message) => match message {
                ShareMessage::Ls => {
                    let shares = state.read().await.shares_dto();
                    Ok(ServerResponse::LsShares(shares))
                }
                ShareMessage::Remove { name } => Ok(state.write().await.remove_share(&name).into()),
                ShareMessage::Share { path, name } => {
                    let path = validate_path(path)?;
                    let name = name.unwrap_or(
                        path.file_name()
                            .ok_or(ServerError::PathNotDir)?
                            .to_string_lossy()
                            .to_string()
                            .parse()?,
                    );
                    let share = Share::new(name, path);
                    state.write().await.create_share(share)?;
                    Ok(ServerResponse::Ok)
                }
            },
        }
    }
    .await;

    let resp = result.unwrap_or_else(ServerResponse::from);
    if let ServerResponse::Err(e) = &resp {
        error!("Error while handling local client request: {e}");
    }
    let _ = stream.write_respone(&resp).await;
}

async fn mount_share(
    path: PathBuf,
    name: FullShareName,
    state: SharedState,
) -> Result<ServerResponse, ServerError> {
    let addr = name.addr.into();
    let lock = state.read().await;
    match lock.get_peers_by_socket().get_by_right(&addr) {
        Some(peer_id) => {
            drop(lock);
            todo!()
        }
        None => {
            let intent = PeerInitReq::JoinShare {
                name: name.name,
                path,
            };
            drop(lock); // Needed to not clone state
            let resp = super::peer::call::call_peer(addr, intent, state).await?;
        }
    };

    Ok(ServerResponse::Ok)
}

fn validate_path(path: String) -> Result<PathBuf, ServerError> {
    let path = PathBuf::from(path);
    if path.is_relative() {
        return Err(ServerError::RelativePath);
    }
    if !path.is_dir() {
        return Err(ServerError::PathNotDir);
    }
    Ok(path)
}
