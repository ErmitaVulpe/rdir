use std::path::Path;

use tokio::{
    net::{UnixListener, UnixStream},
    spawn,
};
use tokio_util::time::FutureExt;
use tracing::{debug, error};

use crate::{
    client::IPC_TIMEOUT,
    common::{
        ClientMessage, ConnectMessage, ServerResponse, ShareMessage, ipc::IpcStream,
        shares::ShareLocator,
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
    let message = match stream.read_command().timeout(IPC_TIMEOUT).await {
        Ok(Ok(val)) => val,
        Ok(Err(_)) => {
            error!("Local client connection timed out");
            return;
        }
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
                    validate_path(&path)?;
                    mount_share(path, name, state.clone()).await
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
                    shares: lock.shares().to_owned(),
                })
            }
            ClientMessage::Ping => Ok(ServerResponse::Ok),
            ClientMessage::Share(message) => match message {
                ShareMessage::Ls => {
                    let shares = state.read().await.shares().to_owned();
                    Ok(ServerResponse::LsShares(shares))
                }
                ShareMessage::Remove { name } => Ok(state.write().await.remove_share(&name).into()),
                ShareMessage::Share { path, name } => {
                    validate_path(&path)?;
                    let name = if let Some(name) = name {
                        name
                    } else {
                        let path: &Path = path.as_ref();
                        path.file_name()
                            .ok_or(ServerError::PathNotDir)?
                            .to_string_lossy()
                            .to_string()
                            .parse()?
                    };
                    let share = Share::new(name, path);
                    debug!("WAITING FOR LOCK");
                    let mut lock = state.write().await;
                    debug!("GOT LOCK");
                    lock.create_share(share)?;
                    debug!("REACHED THIS");
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
    path: String,
    locator: ShareLocator,
    state: SharedState,
) -> Result<ServerResponse, ServerError> {
    let authority = locator.authority;
    let lock = state.read().await;
    match lock.get_peers_by_socket().get_by_right(&authority) {
        Some(peer_id) => {
            drop(lock);
            todo!()
        }
        None => {
            let intent = PeerInitReq::JoinShare {
                name: locator.share_name,
                path,
            };
            drop(lock); // Needed to not clone state
            let resp = super::peer::call::call_peer(authority, intent, state).await?;
        }
    };

    Ok(ServerResponse::Ok)
}

fn validate_path(path: impl AsRef<Path>) -> Result<(), ServerError> {
    let path_ref = path.as_ref();
    if path_ref.is_relative() {
        return Err(ServerError::RelativePath);
    }
    if !path_ref.is_dir() {
        return Err(ServerError::PathNotDir);
    }
    Ok(())
}
