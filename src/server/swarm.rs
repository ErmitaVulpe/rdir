use std::{collections::BTreeMap, net::SocketAddrV4};

use futures::StreamExt;
use libp2p::{
    Multiaddr, PeerId, Swarm,
    multiaddr::Protocol,
    swarm::{DialError, SwarmEvent, dial_opts::DialOpts},
};
use tokio::{
    select,
    sync::{mpsc, oneshot},
};
use tracing::debug;

use crate::server::{SERVER_CANCEL, state::SharedState};

pub async fn drive_swarm(
    mut swarm: Swarm<libp2p_stream::Behaviour>,
    state: SharedState,
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
                    SwarmEvent::Behaviour(()) => unreachable!(),
                    SwarmEvent::ConnectionEstablished { peer_id, connection_id, .. } => {
                        // I have no clue why this type signature is required here
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
                    SwarmEvent::NewExternalAddrOfPeer { peer_id, address} => {
                        let mut iter = address.iter();
                        let ip = match iter.next() {
                            Some(Protocol::Ip4(ip)) => ip,
                            _ => unimplemented!("Just ipv4 supported"),
                        };
                        let port = match iter.next() {
                            Some(Protocol::Tcp(port)) => port,
                            _ => unimplemented!("Just ipv4 supported"),
                        };
                        let new_socket = SocketAddrV4::new(ip, port);
                        state.write().await.update_peer_socket(peer_id, new_socket);
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
