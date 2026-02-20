use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    net::SocketAddrV4,
    path::PathBuf,
};

use bitcode::{Decode, Encode};
use derive_more::{Display, Eq, From, IsVariant, PartialEq};
use libp2p::PeerId;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::common::{
    PeersDto, RemoteSharesDto, ShareDto, SharesDto,
    shares::{CommonShareName, FullShareName, RemotePeerAddr},
};

pub mod error;
use error::*;

#[derive(Debug, Default)]
pub struct State {
    next_peer_id: u32,
    peers: BTreeMap<PeerId, Peer>,
    peers_by_socket: BTreeMap<SocketAddrV4, PeerId>,
    remote_shares: BTreeMap<FullShareName, RemoteShare>,
    shares: BTreeMap<CommonShareName, Share>,
    shutdown_server: CancellationToken,
}

impl State {
    pub fn get_peers(&self) -> &BTreeMap<PeerId, Peer> {
        &self.peers
    }

    pub fn get_peers_by_scoket(&self) -> &BTreeMap<SocketAddrV4, PeerId> {
        &self.peers_by_socket
    }

    pub fn get_shares(&self) -> &BTreeMap<CommonShareName, Share> {
        &self.shares
    }

    pub fn get_remote_shares(&self) -> &BTreeMap<FullShareName, RemoteShare> {
        &self.remote_shares
    }

    pub fn peers_dto(&self) -> PeersDto {
        todo!()
        // let mut data = BTreeMap::new();
        // for (peer_name, peer) in &self.peers {
        //     data.insert(*peer_name, peer.address);
        // }
        //
        // PeersDto(data)
    }

    pub fn remote_shares_dto(&self) -> RemoteSharesDto {
        todo!()
        // let mut data = BTreeMap::new();
        // for (remote_share_name, remote_share) in &self.remote_shares {
        //     let entry = data.entry(remote_share_name.addr.clone());
        //     match entry {
        //         Entry::Vacant(entry) => {
        //             entry.insert(vec![RemoteShareDto::from(remote_share)]);
        //         }
        //         Entry::Occupied(mut entry) => {
        //             entry.get_mut().push(RemoteShareDto::from(remote_share));
        //         }
        //     }
        // }
        //
        // RemoteSharesDto(data)
    }

    pub fn shares_dto(&self) -> SharesDto {
        todo!()
        // SharesDto(self.shares.values().map(ShareDto::from).collect())
    }

    pub fn new_peer_connected_to_share(
        &mut self,
        peer_id: PeerId,
        mut peer: Peer,
        share_name: CommonShareName,
    ) -> Result<PeerId, NewPeerConnectedToShareError> {
        if self.peers_by_socket.contains_key(&peer.address) {
            return Err(RepeatedPeerError.into());
        }

        let share = match self.shares.get_mut(&share_name) {
            Some(val) => val,
            None => return Err(ShareDoesntExistError.into()),
        };

        // all checks passed, now modifying
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
            None => return Err(ShareDoesntExistError.into()),
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
        let share = self
            .shares
            .get_mut(&share_name)
            .ok_or(ShareDoesntExistError)?;
        if !share.participants.remove(&peer_id) {
            return Err(PeerNotUsingShareError.into());
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
        let share = self
            .shares
            .get_mut(&share_name)
            .ok_or(ShareDoesntExistError)?;
        if !share.participants.remove(&peer_id) {
            return Err(PeerNotUsingShareError.into());
        }
        let res = peer.used_shares.remove(&share_name);
        debug_assert!(res);
        peer.notification_tx
            .send(StateNotification::KickedFromShare(share_name))
            .unwrap();
        self.try_drop_peer(peer_id);
        Ok(())
    }

    pub fn remove_peer(&mut self, peer_id: PeerId) -> Result<(), KickPeerFromShareError> {
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
                if peer.used_shares.len() + peer.used_remote_shares.len() == 0 {
                    let (_, peer) = entry.remove_entry();
                    peer.kill_peer.cancel();
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn create_share(&mut self, mut share: Share) -> Result<(), RepeatedShare> {
        let common_name = share.name;
        let entry = self.shares.entry(common_name);
        match entry {
            Entry::Vacant(entry) => {
                share.name = entry.key().clone();
                entry.insert(share);
                Ok(())
            }
            Entry::Occupied(_) => Err(RepeatedShare),
        }
    }

    pub fn remove_share(&mut self, name: &CommonShareName) -> Result<(), ShareDoesntExistError> {
        let (name, share) = self
            .shares
            .remove_entry(name)
            .ok_or(ShareDoesntExistError)?;

        for participant_id in share.participants {
            let peer = self.peers.get_mut(&participant_id).unwrap();
            let res = peer.used_shares.remove(&name);
            debug_assert!(res);
            peer.notification_tx
                .send(StateNotification::KickedFromShare(name.clone()))
                .unwrap();
            self.try_drop_peer(participant_id);
        }

        self.should_server_close();
        Ok(())
    }

    pub fn new_peer_join_remote_share(
        &mut self,
        peer_id: PeerId,
        mut peer: Peer,
        name: FullShareName,
        mount_path: PathBuf,
    ) -> Result<PeerId, RepeatedRemoteShareError> {
        debug_assert!(!self.peers_by_socket.contains_key(&peer.address));
        let Entry::Vacant(entry) = self.remote_shares.entry(name) else {
            return Err(RepeatedRemoteShareError);
        };

        let name = entry.key().clone();
        let remote_share = RemoteShare {
            owner: peer_id,
            name: name.name.clone(),
            mount_path,
        };
        entry.insert(remote_share);

        let res = peer.used_remote_shares.insert(name);
        debug_assert!(res);
        let res = self.peers_by_socket.insert(peer.address, peer_id);
        debug_assert!(res.is_none());
        let res = self.peers.insert(peer_id, peer);
        debug_assert!(res.is_none());
        Ok(peer_id)
    }

    pub fn join_remote_share(
        &mut self,
        peer_id: PeerId,
        name: FullShareName,
        mount_path: PathBuf,
    ) -> Result<(), RepeatedRemoteShareError> {
        let Entry::Vacant(entry) = self.remote_shares.entry(name) else {
            return Err(RepeatedRemoteShareError);
        };

        let name = entry.key().clone();
        let remote_share = RemoteShare {
            owner: peer_id,
            name: name.name.clone(),
            mount_path,
        };
        entry.insert(remote_share);

        let res = self
            .peers
            .get_mut(&peer_id)
            .unwrap()
            .used_remote_shares
            .insert(name);
        debug_assert!(res);
        Ok(())
    }

    pub fn exit_remote_share(
        &mut self,
        peer_id: PeerId,
        remote_share_name: FullShareName,
    ) -> Result<(), ExitPeerShareError> {
        let peer = self.peers.get_mut(&peer_id).unwrap();
        if !peer.used_remote_shares.remove(&remote_share_name) {
            return Err(NoSuchRemoteShareError.into());
        }

        let _remote_share = self.remote_shares.remove(&remote_share_name).unwrap();
        self.try_drop_peer(peer_id);
        self.should_server_close();
        Ok(())
    }

    pub fn should_server_close(&self) {
        if self.peers.is_empty() && self.shares.is_empty() {
            self.shutdown_server.cancel();
        }
    }
}

#[derive(Clone, Debug)]
pub struct Peer {
    pub address: SocketAddrV4,
    kill_peer: CancellationToken,
    notification_tx: mpsc::UnboundedSender<StateNotification>,
    used_remote_shares: BTreeSet<FullShareName>,
    used_shares: BTreeSet<CommonShareName>,
}

impl Peer {
    pub fn new(
        address: SocketAddrV4,
        notification_tx: mpsc::UnboundedSender<StateNotification>,
        kill_peer: CancellationToken,
    ) -> Self {
        Self {
            address,
            kill_peer,
            notification_tx,
            used_remote_shares: Default::default(),
            used_shares: Default::default(),
        }
    }
}

#[derive(Debug)]
pub struct Share {
    pub name: CommonShareName,
    pub path: PathBuf,
    pub participants: BTreeSet<PeerId>,
}

impl Share {
    pub fn new(name: CommonShareName, path: PathBuf) -> Self {
        Self {
            name,
            path,
            participants: Default::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RemoteShare {
    owner: PeerId,
    pub name: CommonShareName,
    pub mount_path: PathBuf,
}

#[derive(Encode, Decode, Clone, Debug, Display, From, IsVariant, PartialEq, Eq)]
pub enum StateNotification {
    KickedFromShare(CommonShareName),
}
