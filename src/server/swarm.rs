use std::collections::BTreeMap;

use futures::StreamExt;
use libp2p::{
    Multiaddr, PeerId, Swarm,
    swarm::{DialError, SwarmEvent, dial_opts::DialOpts},
};
use tokio::{
    select,
    sync::{mpsc, oneshot},
};
use tracing::debug;

use crate::server::SERVER_CANCEL;

pub async fn drive_swarm(
    mut swarm: Swarm<libp2p_stream::Behaviour>,
    mut command_rx: mpsc::Receiver<SwarmCommand>,
) {
    let mut awaiting_dials = BTreeMap::new();

    // main loop
    loop {
        select! {
            _ = SERVER_CANCEL.cancelled() => {
                break;
            },
            event = swarm.next() => {
                let event = event.unwrap();
                debug!("Swarm event: {event:?}");
                match event {
                    SwarmEvent::Behaviour(_) => todo!(),
                    SwarmEvent::ConnectionEstablished { peer_id, connection_id, .. } => {
                        // I have no clue why this type signature here is required
                        // but not in OutgoingConnectionError (rust 1.92.0)
                        let sender: oneshot::Sender<_> = awaiting_dials.remove(&connection_id)
                            .expect("Got ConnectionEstablished without a dial call");
                        let _ = sender.send(Ok(peer_id));
                    },
                    SwarmEvent::OutgoingConnectionError { connection_id, error, .. } => {
                        let sender = awaiting_dials.remove(&connection_id)
                            .expect("Got ConnectionEstablished without a dial call");
                        let _ = sender.send(Err(error));
                    },
                    _ => {}
                }
            },
            command = command_rx.recv() => {
                match command {
                    None => unreachable!("Swarm events never terminates"),
                    Some(command) => match command {
                        SwarmCommand::Dial{ addr, resp } => {
                            let dial_opts = DialOpts::from(addr);
                            let conn_id = dial_opts.connection_id();
                            match swarm.dial(dial_opts) {
                                Ok(_) => {
                                    awaiting_dials.insert(conn_id, resp);
                                },
                                Err(err) => {
                                    let _ = resp.send(Err(err));
                                },
                            }
                        },
                        SwarmCommand::DisconnectPeer { peer_id, resp } => {
                            let _ = resp.send(swarm.disconnect_peer_id(peer_id));
                        },
                    },
                }
            },
        };
    }

    drop(command_rx);
    drop(awaiting_dials);

    // shutdown loop
    while swarm.network_info().num_peers() != 0 {
        swarm.next().await;
    }
}

#[derive(Debug)]
pub enum SwarmCommand {
    Dial {
        addr: Multiaddr,
        resp: oneshot::Sender<Result<PeerId, DialError>>,
    },
    DisconnectPeer {
        peer_id: PeerId,
        resp: oneshot::Sender<Result<(), ()>>,
    },
}
