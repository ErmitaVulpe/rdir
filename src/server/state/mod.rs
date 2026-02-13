use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    net::SocketAddrV4,
    path::PathBuf,
};

use bitcode::{Decode, Encode};
use derive_more::{Display, Eq, Error, From, IsVariant, PartialEq};
use smol::channel::Sender;

use crate::common::shares::{CommonShareName, FullShareName};

#[derive(Debug, Default)]
pub struct State {
    next_peer_id: u32,
    peers: BTreeMap<PeerId, Peer>,
    peers_by_socket: BTreeMap<SocketAddrV4, PeerId>,
    shares: BTreeMap<CommonShareName, Share>,
    connections: BTreeMap<FullShareName, Connection>,
}

/// Helper macro to generate a new PeerId
/// Sometimes I want to create a new PeerId while already holding a ref mut to
/// another field of the State, hence another method does not work
macro_rules! new_peer_id {
    ($state:expr) => {
        loop {
            let id = $state.next_peer_id;
            $state.next_peer_id = $state.next_peer_id.wrapping_add(1);
            let peer_id = PeerId(id);
            if !$state.peers.contains_key(&peer_id) {
                break peer_id;
            }
        }
    };
}

impl State {
    pub fn new_peer_connected_to_share(
        &mut self,
        mut peer: Peer,
        share_name: CommonShareName,
    ) -> Result<PeerId, NewPeerConnectedToShareError> {
        if self.peers_by_socket.contains_key(&peer.address) {
            return Err(RepeatedPeer.into());
        }

        let share = match self.shares.get_mut(&share_name) {
            Some(val) => val,
            None => return Err(ShareDoesntExist.into()),
        };

        // all checks passed, now modifying
        let peer_id = new_peer_id!(self);
        peer.used_shares.insert(share_name);
        let res = self.peers_by_socket.insert(peer.address, peer_id);
        debug_assert!(res.is_none());
        let res = self.peers.insert(peer_id, peer);
        debug_assert!(res.is_none());
        let res = share.participants.insert(peer_id);
        debug_assert!(res);
        Ok(peer_id)
    }

    /// Must not be called after peer was dropped
    pub fn peer_connected_to_share(
        &mut self,
        peer_id: PeerId,
        share_name: CommonShareName,
    ) -> Result<(), PeerConnectedToShareError> {
        let share = match self.shares.get_mut(&share_name) {
            Some(val) => val,
            None => return Err(ShareDoesntExist.into()),
        };

        self.peers
            .get_mut(&peer_id)
            .unwrap()
            .used_shares
            .insert(share_name);
        let res = share.participants.insert(peer_id);
        debug_assert!(res);
        Ok(())
    }

    /// Must not be called after peer was dropped
    pub fn peer_disconnected_from_share(
        &mut self,
        peer_id: PeerId,
        share_name: CommonShareName,
    ) -> Result<(), PeerDisconnectedFromShareError> {
        let peer = self.peers.get_mut(&peer_id).unwrap();
        let share = self.shares.get_mut(&share_name).ok_or(ShareDoesntExist)?;
        if !share.participants.remove(&peer_id) {
            return Err(PeerNotUsingShare.into());
        }
        let res = peer.used_shares.remove(&share_name);
        debug_assert!(res);
        self.try_drop_peer(peer_id);
        Ok(())
    }

    pub fn kick_peer_from_share(
        &mut self,
        peer_id: PeerId,
        share_name: CommonShareName,
    ) -> Result<(), KickPeerFromShareError> {
        let peer = self.peers.get_mut(&peer_id).unwrap();
        let share = self.shares.get_mut(&share_name).ok_or(ShareDoesntExist)?;
        if !share.participants.remove(&peer_id) {
            return Err(PeerNotUsingShare.into());
        }
        let res = peer.used_shares.remove(&share_name);
        debug_assert!(res);
        peer.notification_tx
            .try_send(StateNotification::KickedFromShare(share_name))
            .unwrap();
        self.try_drop_peer(peer_id);
        Ok(())
    }

    pub fn remove_peer(&mut self, peer_id: PeerId) -> Result<(), RemovePeerError> {
        todo!()
    }

    /// removes a peer if it can
    /// must not be called after peer was dropped
    fn try_drop_peer(&mut self, peer_id: PeerId) -> bool {
        let entry = self.peers.entry(peer_id);
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

    pub fn add_share(&mut self, share: Share) -> Result<(), RepeatedShare> {
        let common_name = share.name.clone();
        let entry = self.shares.entry(common_name);
        match entry {
            Entry::Vacant(entry) => {
                entry.insert(share);
                Ok(())
            }
            Entry::Occupied(_) => Err(RepeatedShare),
        }
    }

    pub fn remove_share(&mut self, name: &CommonShareName) -> Result<(), RemoveShareError> {
        let (name, share) = self.shares.remove_entry(name).ok_or(ShareDoesntExist)?;

        for participant_id in share.participants {
            let peer = self.peers.get_mut(&participant_id).unwrap();
            let res = peer.used_shares.remove(&name);
            assert!(res);
            peer.notification_tx
                .try_send(StateNotification::KickedFromShare(name.clone()))
                .unwrap();
            self.try_drop_peer(participant_id);
        }

        Ok(())
    }
}

#[derive(Encode, Decode, Debug, Display, Error, PartialEq, Eq)]
#[display("Specified share doesnt exist")]
pub struct ShareDoesntExist;

#[derive(Encode, Decode, Debug, Display, Error, PartialEq, Eq)]
#[display("Specified peer doesnt exist")]
pub struct PeerDoesntExist;

#[derive(Encode, Decode, Debug, Display, Error, PartialEq, Eq)]
#[display("Specified peer already exists")]
pub struct RepeatedPeer;

#[derive(Encode, Decode, Debug, Display, Error, PartialEq, Eq)]
#[display("Peer isnt connected to this share")]
pub struct PeerNotUsingShare;

#[derive(Encode, Decode, Debug, Display, Error, From, PartialEq, Eq, IsVariant)]
#[display("New peer failed to connect to a share")]
pub enum NewPeerConnectedToShareError {
    RepeatedPeer(RepeatedPeer),
    ShareDoesntExist(ShareDoesntExist),
}

#[derive(Encode, Decode, Debug, Display, Error, From, PartialEq, Eq, IsVariant)]
#[display("Peer failed to connect to a share")]
pub enum PeerConnectedToShareError {
    PeerDoesntExist(PeerDoesntExist),
    ShareDoesntExist(ShareDoesntExist),
}

#[derive(Encode, Decode, Debug, Display, Error, From, PartialEq, Eq, IsVariant)]
#[display("Couldnt disconnect peer from a share")]
pub enum PeerDisconnectedFromShareError {
    PeerNotUsingShare(PeerNotUsingShare),
    ShareDoesntExist(ShareDoesntExist),
}

#[derive(Encode, Decode, Debug, Display, Error, From, PartialEq, Eq, IsVariant)]
#[display("Failed to kick a peer")]
pub enum KickPeerFromShareError {
    PeerNotUsingShare(PeerNotUsingShare),
    ShareDoesntExist(ShareDoesntExist),
}

#[derive(Encode, Decode, Debug, Display, Error, From, PartialEq, Eq, IsVariant)]
#[display("Failed to remove a peer")]
pub enum RemovePeerError {}

#[derive(Encode, Decode, Debug, Display, Error, PartialEq, Eq)]
#[display("Share with this name already exists")]
pub struct RepeatedShare;

#[derive(Encode, Decode, Debug, Display, Error, From, PartialEq, Eq, IsVariant)]
#[display("Failed to add a share")]
pub enum AddShareError {}

#[derive(Encode, Decode, Debug, Display, Error, From, PartialEq, Eq, IsVariant)]
#[display("Failed to remove a share")]
pub enum RemoveShareError {
    ShareDoesntExist(ShareDoesntExist),
}

#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeerId(u32);

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

#[derive(Debug)]
pub struct Share {
    pub name: CommonShareName,
    pub path: PathBuf,
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

#[derive(Clone, Debug)]
pub struct Connection {
    owner: PeerId,
    name: FullShareName,
    mount_path: PathBuf,
}

#[derive(Encode, Decode, Clone, Debug, Display, From, IsVariant, PartialEq, Eq)]
pub enum StateNotification {
    KickedFromShare(CommonShareName),
}

#[cfg(test)]
mod tests {
    use smol::channel::{Receiver, unbounded};

    use crate::server::NETWORK_PORT;

    use super::*;

    impl State {
        fn integrity_check(&self) {
            // 1. validate peers
            for (peer_id, peer) in &self.peers {
                for share_name in &peer.used_shares {
                    let share = self.shares.get(share_name).unwrap();
                    assert!(share.participants.contains(peer_id));
                }

                for connection_name in &peer.used_connections {
                    let connection = self.connections.get(connection_name).unwrap();
                    assert_eq!(&connection.owner, peer_id);
                }
            }

            // 2. validate shares
            for (share_name, share) in &self.shares {
                for peer_id in &share.participants {
                    let peer = self.peers.get(peer_id).unwrap();
                    assert!(peer.used_shares.contains(share_name));
                }
            }

            // 3. validate connections
            for (connection_name, connection) in &self.connections {
                let peer = self.peers.get(&connection.owner).unwrap();
                assert!(peer.used_connections.contains(connection_name));
            }
        }
    }

    /// test utility
    fn new_peer(id: u8) -> (Peer, Receiver<()>, Receiver<StateNotification>) {
        let address = SocketAddrV4::new([id; 4].into(), NETWORK_PORT);
        let (shutdown_tx, shutdown_rx) = unbounded();
        let (notification_tx, notification_rx) = unbounded();
        let peer = Peer::new(address, shutdown_tx, notification_tx);
        (peer, shutdown_rx, notification_rx)
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
        assert_eq!(state.add_share(share2), Err(RepeatedShare));
        assert!(state.add_share(share3).is_ok());
        assert_eq!(state.shares.len(), 2);
        state.integrity_check();

        state.remove_share(&a_name).unwrap();
        state.remove_share(&b_name).unwrap();
        assert!(
            state
                .remove_share(&c_name)
                .unwrap_err()
                .is_share_doesnt_exist()
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
        state.integrity_check();

        let peer_id = state
            .new_peer_connected_to_share(peer, share_name1.clone())
            .unwrap();
        let peer_ref = state.peers.get(&peer_id).unwrap();
        assert_eq!(peer_ref.used_connections.len(), 0);
        assert_eq!(peer_ref.used_shares.len(), 1);
        state.integrity_check();
        state
            .peer_connected_to_share(peer_id, share_name2.clone())
            .unwrap();
        let peer_ref = state.peers.get(&peer_id).unwrap();
        assert_eq!(peer_ref.used_connections.len(), 0);
        assert_eq!(peer_ref.used_shares.len(), 2);
        state.integrity_check();
        // Now peer uses 2 shares

        state
            .peer_disconnected_from_share(peer_id, share_name1.clone())
            .unwrap();
        assert!(state.peers.get(&peer_id).is_some());
        state.integrity_check();

        state
            .peer_disconnected_from_share(peer_id, share_name1.clone())
            .unwrap_err();
        state.integrity_check();
        assert!(shutdown_rx.try_recv().is_err());

        state
            .peer_disconnected_from_share(peer_id, share_name2.clone())
            .unwrap();
        assert!(state.peers.get(&peer_id).is_none());
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
        state.integrity_check();

        let peer_id = state
            .new_peer_connected_to_share(peer, share_name1.clone())
            .unwrap();
        state
            .peer_connected_to_share(peer_id, share_name2.clone())
            .unwrap();
        state.integrity_check();

        state.remove_share(&share_name1).unwrap();
        state.integrity_check();
        assert!(state.peers.get(&peer_id).is_some());
        assert!(notification_rx.try_recv().unwrap().is_kicked_from_share());
        assert!(shutdown_rx.try_recv().is_err());

        state.remove_share(&share_name2).unwrap();
        state.integrity_check();
        assert!(state.peers.get(&peer_id).is_none());
        assert!(notification_rx.try_recv().unwrap().is_kicked_from_share());
        assert!(shutdown_rx.try_recv().is_ok());
    }
}
