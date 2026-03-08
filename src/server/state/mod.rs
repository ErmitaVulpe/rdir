use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    net::SocketAddrV4,
    path::PathBuf,
    sync::Arc,
};

use bimap::BiBTreeMap;
use snowstorm::Keypair;
use tokio::{
    io,
    sync::{RwLock, mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

use crate::{
    common::{
        PeersDto, RemoteShareDto, RemoteSharesDto, ShareDto, SharesDto,
        shares::{CommonShareName, FullShareName, ShareName},
    },
    server::{
        SERVER_CANCEL,
        peer::{PeerId, PeerInitResp, call::PeerInitReq},
    },
};

pub mod error;
use error::*;

pub type SharedState = Arc<RwLock<State>>;

pub struct State {
    peers: BTreeMap<PeerId, Peer>,
    peers_by_socket: BiBTreeMap<PeerId, SocketAddrV4>,
    remote_shares: BTreeMap<FullShareName, RemoteShare>,
    shares: BTreeMap<CommonShareName, Share>,

    identity: Keypair,
}

impl State {
    pub fn new(identity: Keypair) -> Self {
        Self {
            peers: Default::default(),
            peers_by_socket: Default::default(),
            remote_shares: Default::default(),
            shares: Default::default(),
            identity,
        }
    }

    pub fn get_peers_by_socket(&self) -> &BiBTreeMap<PeerId, SocketAddrV4> {
        &self.peers_by_socket
    }

    pub fn get_identity(&self) -> &Keypair {
        &self.identity
    }

    pub fn get_share_names(&self) -> Vec<CommonShareName> {
        self.shares.keys().cloned().collect()
    }

    pub fn update_peer_socket(&mut self, peer_id: PeerId, new_socket: SocketAddrV4) {
        let res = self.peers_by_socket.insert(peer_id, new_socket);
        debug_assert!(matches!(res, bimap::Overwritten::Left(_, _)));
    }

    pub fn peers_dto(&self) -> PeersDto {
        let mut data = BTreeMap::new();
        for (peer_id, peer) in &self.peers {
            data.insert(peer_id.clone(), peer.address);
        }

        PeersDto(data)
    }

    pub fn remote_shares_dto(&self) -> RemoteSharesDto {
        let mut data = BTreeMap::new();
        for (remote_share_name, remote_share) in &self.remote_shares {
            let entry = data.entry(remote_share_name.addr.clone());
            match entry {
                Entry::Vacant(entry) => {
                    entry.insert(vec![RemoteShareDto::from(remote_share)]);
                }
                Entry::Occupied(mut entry) => {
                    entry.get_mut().push(RemoteShareDto::from(remote_share));
                }
            }
        }

        RemoteSharesDto(data)
    }

    pub fn shares_dto(&self) -> SharesDto {
        SharesDto(self.shares.values().map(ShareDto::from).collect())
    }

    pub fn new_peer_connected_to_share(
        &mut self,
        peer_id: PeerId,
        peer_addr: SocketAddrV4,
        share_name: CommonShareName,
    ) -> Result<mpsc::Receiver<PeerNotification>, NewPeerConnectedToShareError> {
        if self.peers_by_socket.contains_right(&peer_addr) {
            return Err(RepeatedPeerError.into());
        }

        let share = match self.shares.get_mut(&share_name) {
            Some(val) => val,
            None => return Err(ShareDoesntExistError.into()),
        };

        // all checks passed, now modifying
        let (notification_tx, notification_rx) = mpsc::channel(4);
        let mut peer = Peer::new(peer_addr, notification_tx);
        peer.used_shares.insert(share_name);
        let res = self
            .peers_by_socket
            .insert_no_overwrite(peer_id.clone(), peer.address);
        debug_assert!(res.is_ok());
        let res = self.peers.insert(peer_id.clone(), peer);
        debug_assert!(res.is_none());
        let res = share.participants.insert(peer_id.clone());
        debug_assert!(res);
        Ok(notification_rx)
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
                    peer.kill_peer_conn.cancel();
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
            self.try_drop_peer(participant_id);
        }

        self.try_close_server();
        Ok(())
    }

    pub fn try_join_remote_share_by_full_name(
        &mut self,
        full_share_name: FullShareName,
        mount_path: PathBuf,
    ) -> Result<(), ()> {
        let peer_id = self
            .peers_by_socket
            .get_by_right(&full_share_name.addr.into());

        todo!()
    }

    pub fn new_peer_join_remote_share(
        &mut self,
        peer_id: PeerId,
        peer_addr: SocketAddrV4,
        name: CommonShareName,
        mount_path: PathBuf,
    ) -> Result<mpsc::Receiver<PeerNotification>, NewPeerJoinRemoteShareError> {
        if self.peers_by_socket.contains_right(&peer_addr) {
            return Err(RepeatedPeerError.into());
        }

        let full_name = FullShareName::from((peer_addr, name));
        let Entry::Vacant(entry) = self.remote_shares.entry(full_name) else {
            return Err(RepeatedRemoteShareError.into());
        };

        let name = entry.key().clone();
        let remote_share = RemoteShare {
            owner: peer_id.clone(),
            name: name.name.clone(),
            mount_path,
        };
        entry.insert(remote_share);

        let (notification_tx, notification_rx) = mpsc::channel(4);
        let mut peer = Peer::new(peer_addr, notification_tx);
        let res = peer.used_remote_shares.insert(name.name);
        debug_assert!(res);
        let res = self
            .peers_by_socket
            .insert_no_overwrite(peer_id.clone(), peer.address);
        debug_assert!(res.is_ok());
        let res = self.peers.insert(peer_id.clone(), peer);
        debug_assert!(res.is_none());
        Ok(notification_rx)
    }

    pub fn join_remote_share(
        &mut self,
        peer_id: PeerId,
        name: CommonShareName,
        mount_path: PathBuf,
    ) -> Result<(), PeerJoinRemoteShareError> {
        let Some(peer) = self.peers.get_mut(&peer_id) else {
            return Err(PeerDoesntExistError.into());
        };

        let full_name = FullShareName::from((peer.address, name));
        let Entry::Vacant(remote_share_entry) = self.remote_shares.entry(full_name) else {
            return Err(RepeatedRemoteShareError.into());
        };

        let name = remote_share_entry.key().clone();
        if !peer.used_shares.insert(name.name.clone()) {
            return Err(RepeatedRemoteShareError.into());
        }

        remote_share_entry.insert(RemoteShare {
            owner: peer_id.clone(),
            name: name.name,
            mount_path,
        });
        Ok(())
    }

    pub fn exit_remote_share(
        &mut self,
        remote_share_name: ShareName,
    ) -> Result<(), ExitPeerShareError> {
        let (remote_share_name, remote_share) = match remote_share_name {
            ShareName::Common(common_share_name) => {
                // lookup is O(n), could search by just a range since this is BTree
                let mut iter = self
                    .remote_shares
                    .iter()
                    .filter(|(k, _)| k.name == common_share_name);
                let peer_entry = iter.next().ok_or(NoSuchRemoteShareError)?;
                if iter.next().is_some() {
                    return Err(RemoteShareNameAmbiguousError.into());
                }
                self.remote_shares
                    .remove_entry(&peer_entry.0.clone())
                    .unwrap()
            }
            ShareName::Full(full_share_name) => self
                .remote_shares
                .remove_entry(&full_share_name)
                .ok_or(NoSuchRemoteShareError)?,
        };

        let peer_id = remote_share.owner;
        let peer = self.peers.get_mut(&peer_id).unwrap();
        let res = peer.used_remote_shares.remove(&remote_share_name.name);
        debug_assert!(res);
        self.try_drop_peer(peer_id);
        self.try_close_server();
        Ok(())
    }

    /// Sends a global shutdown signal if server has no peers and no shares
    pub fn try_close_server(&self) {
        if self.peers.is_empty() && self.shares.is_empty() {
            SERVER_CANCEL.cancel();
        }
    }
}

pub struct PeerNotification {
    pub req: PeerInitReq,
    pub resp: Option<oneshot::Sender<PeerInitResp>>,
}

#[derive(Clone, Debug)]
pub struct Peer {
    pub address: SocketAddrV4,
    pub conn_notification: mpsc::Sender<PeerNotification>,
    kill_peer_conn: CancellationToken,
    /// Shares of the peer that we are using
    used_remote_shares: BTreeSet<CommonShareName>,
    /// Our shares that the peer is using
    used_shares: BTreeSet<CommonShareName>,
}

impl Peer {
    fn new(address: SocketAddrV4, notification_tx: mpsc::Sender<PeerNotification>) -> Self {
        Self {
            address,
            conn_notification: notification_tx,
            kill_peer_conn: SERVER_CANCEL.child_token(),
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
