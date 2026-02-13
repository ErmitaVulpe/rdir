use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    net::SocketAddrV4,
    path::PathBuf,
};

use anyhow::Context;
use bitcode::{Decode, Encode};
use derive_more::{Constructor, Display, Eq, Error, From, IsVariant, PartialEq};
use smol::channel::Sender;

use crate::common::shares::{CommonShareName, FullShareName};

#[derive(Debug, Default)]
pub struct State {
    peers: Peers,
    shares: BTreeMap<CommonShareName, Share>,
    connections: BTreeMap<FullShareName, Connection>,
}

impl State {
    pub fn new_peer(&mut self, peer: Peer) -> PeerId {
        self.peers.new_peer(peer)
    }

    pub fn remove_peer(&mut self, peer_id: PeerId) {
        let Some(peer) = self.peers.inner.get(&peer_id) else {
            return;
        };
        let share_iter = peer.used_shares.clone().into_iter();
        let connection_iter = peer.used_connections.clone().into_iter();

        for share in share_iter {
            self.kick_from_share(share, peer_id)
                .context(CorruptedState)
                .unwrap();
        }

        let peer = self
            .peers
            .inner
            .remove(&peer_id)
            .context(CorruptedState)
            .unwrap();
        let _ = peer.shutdown_tx.try_send(());
    }

    pub fn add_share(&mut self, share: Share) -> Result<(), AddShareError> {
        let common_name = share.name.clone();
        let entry = self.shares.entry(common_name);
        match entry {
            Entry::Vacant(entry) => {
                entry.insert(share);
                Ok(())
            }
            Entry::Occupied(_) => Err(AddShareError),
        }
    }

    pub fn peer_connected_to_share(
        &mut self,
        name: CommonShareName,
        peer_id: PeerId,
    ) -> Result<(), ConnectToShareError> {
        let entry = self.shares.entry(name);
        match entry {
            Entry::Vacant(_) => Err(ConnectToShareError::DoesntExist),
            Entry::Occupied(mut entry) => match entry.get_mut().participants.insert(peer_id) {
                true => {
                    let inserted = self
                        .peers
                        .inner
                        .get_mut(&peer_id)
                        .context(PeerAlreadyGCed)
                        .unwrap()
                        .used_shares
                        .insert(entry.key().clone());
                    assert!(inserted);
                    Ok(())
                }
                false => Err(ConnectToShareError::AlreadyConnected),
            },
        }
    }

    pub fn peer_disconnected_from_share(
        &mut self,
        name: CommonShareName,
        peer_id: PeerId,
    ) -> Result<(), DisconnectFromShareError> {
        let entry = self.shares.entry(name);
        match entry {
            Entry::Vacant(_) => Err(DisconnectFromShareError::DoesntExist),
            Entry::Occupied(mut entry) => match entry.get_mut().participants.remove(&peer_id) {
                true => {
                    let peer = self
                        .peers
                        .inner
                        .get_mut(&peer_id)
                        .context(PeerAlreadyGCed)
                        .unwrap();
                    assert!(peer.used_shares.remove(entry.key()));
                    self.peers.try_remove_peer(peer_id);
                    Ok(())
                }
                false => Err(DisconnectFromShareError::NotConnected),
            },
        }
    }

    pub fn kick_from_share(
        &mut self,
        name: CommonShareName,
        peer_id: PeerId,
    ) -> Result<(), KickFromShareError> {
        let _ = self
            .peers
            .inner
            .get(&peer_id)
            .ok_or(KickFromShareError::NotConnected)?
            .notification_tx
            .try_send(StateNotification::KickedFromShare(name.clone()));
        self.peer_disconnected_from_share(name, peer_id)?;
        Ok(())
    }

    pub fn remove_share(&mut self, name: &CommonShareName) -> Result<(), RemoveShareError> {
        let (k, v) = self
            .shares
            .remove_entry(name)
            .ok_or(RemoveShareError::DoesntExist)?;

        for peer_id in v.participants.into_iter() {
            let peer = self
                .peers
                .inner
                .get_mut(&peer_id)
                .context(CorruptedState)
                .unwrap();

            let _ = peer
                .notification_tx
                .try_send(StateNotification::KickedFromShare(k.clone()));
            let removed = peer.used_shares.remove(&k);
            assert!(removed);

            self.peers.try_remove_peer(peer_id);
        }

        Ok(())
    }
}

#[cfg(test)]
impl State {
    fn integrity_check(&self) {
        // 1. validate peers
        for (peer_id, peer) in &self.peers.inner {
            for share_name in &peer.used_shares {
                let share = self.shares.get(share_name).unwrap();
                assert!(share.participants.contains(peer_id));
            }

            for connection_name in &peer.used_connections {
                let connection = self.connections.get(connection_name).unwrap();
                assert!(connection.participants.contains(peer_id));
            }
        }

        // 2. validate shares
        for (share_name, share) in &self.shares {
            for peer_id in &share.participants {
                let peer = self.peers.inner.get(peer_id).unwrap();
                assert!(peer.used_shares.contains(share_name));
            }
        }

        // 3. validate connections
        for (connection_name, connection) in &self.connections {
            for peer_id in &connection.participants {
                let peer = self.peers.inner.get(peer_id).unwrap();
                assert!(peer.used_connections.contains(connection_name));
            }
        }
    }
}

#[derive(Debug, Display, Error, PartialEq, Eq)]
#[display("State is corrupted")]
pub struct CorruptedState;

#[derive(Debug, Display, Error, PartialEq, Eq)]
#[display("Share name already taken")]
pub struct AddShareError;

#[derive(Debug, Display, Error, PartialEq, Eq)]
pub enum ConnectToShareError {
    #[display("Tried to connect to a share that doesnt exist")]
    DoesntExist,
    #[display("Already connected to this share")]
    AlreadyConnected,
}

#[derive(Debug, Display, Error, PartialEq, Eq)]
pub enum DisconnectFromShareError {
    #[display("Tried to disconnect from a share that doesnt exist")]
    DoesntExist,
    #[display("Not connected to this share")]
    NotConnected,
}

#[derive(Debug, Display, Error, PartialEq, Eq)]
pub enum KickFromShareError {
    #[display("Tried to kick from a share that doesnt exist")]
    DoesntExist,
    #[display("Peer wasnt connected to this share")]
    NotConnected,
}

impl From<DisconnectFromShareError> for KickFromShareError {
    fn from(value: DisconnectFromShareError) -> Self {
        match value {
            DisconnectFromShareError::DoesntExist => Self::DoesntExist,
            DisconnectFromShareError::NotConnected => Self::NotConnected,
        }
    }
}

#[derive(Debug, Display, Error, PartialEq, Eq)]
pub enum RemoveShareError {
    #[display("Tried to remove a share that doesnt exist")]
    DoesntExist,
}

/// Automatically sends a shutdown msg on peer drop
#[derive(Debug, Default)]
struct Peers {
    next_peer_id: u32,
    /// key is the number of mutual shares
    inner: BTreeMap<PeerId, Peer>,
}

impl Peers {
    fn new_peer(&mut self, peer: Peer) -> PeerId {
        loop {
            let id = self.next_peer_id;
            self.next_peer_id = self.next_peer_id.wrapping_add(1);
            let peer_id = PeerId::from(id);
            if let Entry::Vacant(e) = self.inner.entry(peer_id) {
                e.insert(peer);
                return peer_id;
            }
        }
    }

    /// Removes only if no common shares are left
    fn try_remove_peer(&mut self, id: PeerId) -> bool {
        let entry = self.inner.entry(id);
        match entry {
            Entry::Vacant(_) => unreachable!("Tried to remove a non existing peer"),
            Entry::Occupied(entry) => {
                let peer = entry.get();
                if peer.used_shares.len() + peer.used_connections.len() == 0 {
                    let _ = entry.remove_entry().1.shutdown_tx.try_send(());
                    true
                } else {
                    false
                }
            }
        }
    }

    fn get_inner(&self) -> &BTreeMap<PeerId, Peer> {
        &self.inner
    }
}

#[derive(Debug)]
pub struct Share {
    name: CommonShareName,
    path: PathBuf,
    participants: BTreeSet<PeerId>,
}

impl Share {
    fn new(name: CommonShareName, path: PathBuf) -> Self {
        Self {
            name,
            path,
            participants: Default::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, From, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeerId(u32);

#[derive(Debug, Display)]
#[display(
    "Tried to do an operation with a peer that already got garbage collected,\ntry reordering operations"
)]
struct PeerAlreadyGCed;

#[derive(Clone, Debug)]
pub struct Peer {
    pub address: SocketAddrV4,
    used_connections: BTreeSet<FullShareName>,
    used_shares: BTreeSet<CommonShareName>,
    shutdown_tx: Sender<()>,
    notification_tx: Sender<StateNotification>,
}

impl Peer {
    fn new(
        address: SocketAddrV4,
        shutdown_tx: Sender<()>,
        notification_tx: Sender<StateNotification>,
    ) -> Self {
        Self {
            address,
            used_connections: Default::default(),
            used_shares: Default::default(),
            shutdown_tx,
            notification_tx,
        }
    }
}

#[derive(Encode, Decode, Clone, Debug, Display, From, IsVariant, PartialEq, Eq)]
pub enum StateNotification {
    KickedFromShare(CommonShareName),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    name: FullShareName,
    mount_path: PathBuf,
    participants: BTreeSet<PeerId>,
}

impl Connection {
    pub fn new(name: FullShareName, mount_path: PathBuf) -> Self {
        Self {
            name,
            mount_path,
            participants: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use smol::channel::{Receiver, unbounded};

    use crate::server::NETWORK_PORT;

    use super::*;

    fn new_peer(id: u8) -> (Peer, Receiver<()>, Receiver<StateNotification>) {
        let address = SocketAddrV4::new([id; 4].into(), NETWORK_PORT);
        let (shutdown_tx, shutdown_rx) = unbounded();
        let (notification_tx, notification_rx) = unbounded();
        let peer = Peer::new(address, shutdown_tx, notification_tx);
        (peer, shutdown_rx, notification_rx)
    }

    #[test]
    fn managing_peers() {
        let mut state = State::default();
        let (peer1, shutdown_rx1, _) = new_peer(1);
        let peer_id1 = state.new_peer(peer1);
        assert_eq!(peer_id1.0, 0);
        assert_eq!(state.peers.next_peer_id, 1);
        state.integrity_check();

        state.peers.next_peer_id = u32::MAX;

        let (peer2, shutdown_rx2, _) = new_peer(2);
        let peer_id2 = state.new_peer(peer2);
        assert_eq!(peer_id2.0, u32::MAX);
        assert_eq!(state.peers.next_peer_id, 0);
        state.integrity_check();

        let (peer3, shutdown_rx3, _) = new_peer(3);
        let peer_id3 = state.new_peer(peer3);
        assert_eq!(peer_id3.0, 1);
        assert_eq!(state.peers.next_peer_id, 2);
        state.integrity_check();

        state.remove_peer(peer_id1);
        assert!(shutdown_rx1.try_recv().is_ok());
        assert!(shutdown_rx2.try_recv().is_err());
        assert!(shutdown_rx3.try_recv().is_err());
        assert_eq!(state.peers.inner.len(), 2);
        state.integrity_check();
    }

    #[test]
    fn managing_shares() {
        let mut state = State::default();
        let a_name: CommonShareName = "A".parse().unwrap();
        let b_name: CommonShareName = "B".parse().unwrap();
        let c_name: CommonShareName = "C".parse().unwrap();
        let share1 = Share::new(a_name.clone(), PathBuf::from("/1"));
        let share2 = Share::new(a_name.clone(), PathBuf::from("/2"));
        let share3 = Share::new(b_name.clone(), PathBuf::from("/2"));
        state.integrity_check();

        assert!(state.add_share(share1).is_ok());
        assert_eq!(state.add_share(share2), Err(AddShareError));
        assert!(state.add_share(share3).is_ok());
        assert_eq!(state.shares.len(), 2);
        state.integrity_check();

        state.remove_share(&a_name).unwrap();
        state.remove_share(&b_name).unwrap();
        assert_eq!(
            state.remove_share(&c_name),
            Err(RemoveShareError::DoesntExist)
        );
        assert_eq!(state.shares.len(), 0);
        state.integrity_check();
    }

    #[test]
    fn connect_and_disconnect_peer_to_share() {
        let mut state = State::default();
        let share_name1: CommonShareName = "A".parse().unwrap();
        let share1 = Share::new(share_name1.clone(), PathBuf::from("/"));
        let share_name2: CommonShareName = "B".parse().unwrap();
        let share2 = Share::new(share_name2.clone(), PathBuf::from("/"));
        state.add_share(share1).unwrap();
        state.add_share(share2).unwrap();
        let (peer, shutdown_rx, _) = new_peer(1);
        let peer_id = state.new_peer(peer);
        state.integrity_check();

        state
            .peer_connected_to_share(share_name1.clone(), peer_id)
            .unwrap();
        let peer_ref = state.peers.inner.get(&peer_id).unwrap();
        assert_eq!(peer_ref.used_connections.len(), 0);
        assert_eq!(peer_ref.used_shares.len(), 1);
        state.integrity_check();
        state
            .peer_connected_to_share(share_name2.clone(), peer_id)
            .unwrap();
        let peer_ref = state.peers.inner.get(&peer_id).unwrap();
        assert_eq!(peer_ref.used_connections.len(), 0);
        assert_eq!(peer_ref.used_shares.len(), 2);
        state.integrity_check();
        // Now peer uses 2 shares

        state
            .peer_disconnected_from_share(share_name1.clone(), peer_id)
            .unwrap();
        assert!(state.peers.inner.get(&peer_id).is_some());
        state.integrity_check();

        state
            .peer_disconnected_from_share(share_name1.clone(), peer_id)
            .unwrap_err();
        state.integrity_check();
        assert!(shutdown_rx.try_recv().is_err());

        state
            .peer_disconnected_from_share(share_name2.clone(), peer_id)
            .unwrap();
        assert!(state.peers.inner.get(&peer_id).is_none());
        assert!(shutdown_rx.try_recv().is_ok());
        state.integrity_check();
    }

    #[test]
    fn remove_share() {
        let mut state = State::default();
        let share_name1: CommonShareName = "A".parse().unwrap();
        let share1 = Share::new(share_name1.clone(), PathBuf::from("/"));
        let share_name2: CommonShareName = "B".parse().unwrap();
        let share2 = Share::new(share_name2.clone(), PathBuf::from("/"));
        state.add_share(share1).unwrap();
        state.add_share(share2).unwrap();
        let (peer, shutdown_rx, notification_rx) = new_peer(1);
        let peer_id = state.new_peer(peer);
        state.integrity_check();

        state
            .peer_connected_to_share(share_name1.clone(), peer_id)
            .unwrap();
        state
            .peer_connected_to_share(share_name2.clone(), peer_id)
            .unwrap();
        state.integrity_check();

        state.remove_share(&share_name1).unwrap();
        state.integrity_check();
        assert!(state.peers.inner.get(&peer_id).is_some());
        assert!(notification_rx.try_recv().unwrap().is_kicked_from_share());
        assert!(shutdown_rx.try_recv().is_err());

        state.remove_share(&share_name2).unwrap();
        state.integrity_check();
        assert!(state.peers.inner.get(&peer_id).is_none());
        assert!(notification_rx.try_recv().unwrap().is_kicked_from_share());
        assert!(shutdown_rx.try_recv().is_ok());
    }
}
