use tokio::{
    net::{UnixListener, UnixStream},
    spawn,
};
use tracing::{debug, error};

use crate::{
    common::{ClientMessage, ConnectMessage, ServerResponse, ipc::IpcStream},
    server::{SERVER_CANCEL, ServerError, state::SharedState},
};

pub async fn accpet_client(listener: UnixListener, state: SharedState) {
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                spawn(handle_client(stream, state.clone()));
            }
            Err(e) => error!("Error while accepting a local client: {e}"),
        }
    }
}

async fn handle_client(stream: UnixStream, state: SharedState) {
    let mut stream = IpcStream::new_server(stream);
    let message = match stream.read_command().await {
        Ok(val) => val,
        Err(err) => {
            error!("Failed to receive a command from local client {err}");
            return;
        }
    };
    debug!("Client sent: {message:?}");

    let result: Result<ServerResponse, ServerError> = match message {
        ClientMessage::Connect(message) => match message {
            ConnectMessage::Ls => {
                let shares = state.read().await.remote_shares_dto();
                Ok(ServerResponse::LsMountedShares(shares))
            }
            ConnectMessage::Mount { path, name } => todo!(),
            ConnectMessage::Unmount { name } => todo!(),
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
        ClientMessage::Ping => todo!(),
        ClientMessage::Share(message) => todo!(),
    };

    let resp = result
        .inspect_err(|e| error!("Error while handling local client: {e}"))
        .unwrap_or_else(ServerResponse::from);
    let _ = stream.write_respone(&resp).await;
}
