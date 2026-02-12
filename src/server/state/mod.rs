use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    net::Ipv4Addr,
    path::PathBuf,
};

use anyhow::Context;
use bitcode::{Decode, Encode};
use derive_more::{Constructor, Display, Error, From, IsVariant};
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

    pub fn connected_to_share(
        &mut self,
        name: CommonShareName,
        peer: PeerId,
    ) -> Result<(), ConnectToShareError> {
        let entry = self.shares.entry(name);
        match entry {
            Entry::Vacant(_) => Err(ConnectToShareError::DoesntExist),
            Entry::Occupied(mut entry) => match entry.get_mut().participants.insert(peer) {
                true => {
                    self.peers
                        .inner
                        .get_mut(&peer)
                        .context(PeerAlreadyGCed)
                        .unwrap()
                        .common_shares += 1;
                    Ok(())
                }
                false => Err(ConnectToShareError::AlreadyConnected),
            },
        }
    }

    pub fn disconnect_from_share(
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
                    peer.common_shares -= 1;
                    self.peers.try_remove_peer(peer_id);
                    Ok(())
                }
                false => Err(DisconnectFromShareError::NotConnected),
            },
        }
    }

    pub fn remove_share(&mut self, name: CommonShareName) -> Result<(), RemoveShareError> {
        let (k, v) = self
            .shares
            .remove_entry(&name)
            .ok_or(RemoveShareError::DoesntExist)?;

        let _ = v
            .participants
            .into_iter()
            .map(|id| {
                self.peers
                    .inner
                    .get(&id)
                    .context(CorruptedState)
                    .map(|p| {
                        p.notification_tx
                            .try_send(StateNotification::KickedFromShare(k.clone()))
                    })
                    .context(CorruptedState)
                    .and_then(|_| {
                        self.disconnect_from_share(name.clone(), id)
                            .context(CorruptedState)
                    })
                    .unwrap();
            })
            .collect::<()>();
        Ok(())
    }
}

#[derive(Debug, Display, Error)]
#[display("State id corrupted")]
pub struct CorruptedState;

#[derive(Debug, Display, Error)]
#[display("Share name already taken")]
pub struct AddShareError;

#[derive(Debug, Display, Error)]
pub enum ConnectToShareError {
    #[display("Tried to connect to a share that doesnt exist")]
    DoesntExist,
    #[display("Already connected to this share")]
    AlreadyConnected,
}

#[derive(Debug, Display, Error)]
pub enum DisconnectFromShareError {
    #[display("Tried to disconnect from a share that doesnt exist")]
    DoesntExist,
    #[display("Not connected to this share")]
    NotConnected,
}

#[derive(Debug, Display, Error)]
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
    fn try_remove_peer(&mut self, id: PeerId) {
        let entry = self.inner.entry(id);
        match entry {
            Entry::Vacant(_) => unreachable!("Tried to remove a non existing peer"),
            Entry::Occupied(entry) => {
                if entry.get().common_shares == 0 {
                    let _ = entry.remove_entry().1.shutdown_tx.try_send(());
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

#[must_use]
#[derive(Clone, Copy, Debug, From, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeerId(u32);

#[derive(Debug, Display)]
#[display(
    "Tried to do an operation with a peer that already got garbage collected,\ntry reordering operations"
)]
struct PeerAlreadyGCed;

#[derive(Clone, Constructor, Debug)]
pub struct Peer {
    pub common_shares: u32,
    pub address: Ipv4Addr,
    shutdown_tx: Sender<()>,
    notification_tx: Sender<StateNotification>,
}

#[derive(Encode, Decode, Clone, Debug, Display, From, IsVariant, PartialEq, Eq)]
pub enum StateNotification {
    KickedFromShare(CommonShareName),
}

#[derive(Clone, Debug, Constructor)]
pub struct Connection {
    name: FullShareName,
    mount_path: PathBuf,
}
