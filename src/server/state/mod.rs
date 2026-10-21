use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
};

use bimap::BiBTreeMap;
use bitcode::{Decode, Encode};
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::{
    common::{
        PeersDto, RemoteSharesDto,
        shares::{Authority, ShareLocator, ShareName},
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
    peers_by_authority: BiBTreeMap<PeerId, Authority>,
    remote_shares: BTreeMap<ShareLocator, RemoteShare>,
    local_shares: BTreeMap<ShareName, Share>,

    endpoint: quinn::Endpoint,
}

impl State {
    pub fn new(endpoint: quinn::Endpoint) -> Self {
        Self {
            peers: Default::default(),
            peers_by_authority: Default::default(),
            remote_shares: Default::default(),
            local_shares: Default::default(),
            endpoint,
        }
    }

    pub fn get_peers_by_socket(&self) -> &BiBTreeMap<PeerId, Authority> {
        &self.peers_by_authority
    }

    pub fn get_endpoint(&self) -> &quinn::Endpoint {
        &self.endpoint
    }

    pub fn get_share_names(&self) -> Vec<ShareName> {
        self.local_shares.keys().cloned().collect()
    }

    pub fn peers_dto(&self) -> PeersDto {
        let mut data = BTreeMap::new();
        for (peer_id, peer) in &self.peers {
            data.insert(peer_id.clone(), peer.address);
        }

        PeersDto(data)
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

    pub fn shares(&self) -> &BTreeMap<ShareName, Share> {
        &self.local_shares
    }

    /// Must not be called after peer was dropped
    pub fn peer_connected_to_share(
        &mut self,
        peer_id: PeerId,
        share_name: ShareName,
    ) -> Result<(), PeerConnectedToShareError> {
        let share = match self.local_shares.get_mut(&share_name) {
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
        share_name: ShareName,
    ) -> Result<(), PeerDisconnectedFromShareError> {
        let peer = self.peers.get_mut(&peer_id).unwrap();
        let share = self
            .local_shares
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
        share_name: ShareName,
    ) -> Result<(), KickPeerFromShareError> {
        let peer = self.peers.get_mut(&peer_id).unwrap();
        let share = self
            .local_shares
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
        let entry = self.local_shares.entry(common_name);
        match entry {
            Entry::Vacant(entry) => {
                share.name = entry.key().clone();
                entry.insert(share);
                Ok(())
            }
            Entry::Occupied(_) => Err(RepeatedShare),
        }
    }

    pub fn remove_share(&mut self, name: &ShareName) -> Result<(), ShareDoesntExistError> {
        let (name, share) = self
            .local_shares
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

    pub fn exit_remote_share(
        &mut self,
        remote_share_name: ShareName,
    ) -> Result<(), ExitPeerShareError> {
        todo!()
        // let (remote_share_name, remote_share) = match remote_share_name {
        //     ShareName::Common(common_share_name) => {
        //         // lookup is O(n), could search by just a range since this is BTree
        //         let mut iter = self
        //             .remote_shares
        //             .iter()
        //             .filter(|(k, _)| k.name == common_share_name);
        //         let peer_entry = iter.next().ok_or(NoSuchRemoteShareError)?;
        //         if iter.next().is_some() {
        //             return Err(RemoteShareNameAmbiguousError.into());
        //         }
        //         self.remote_shares
        //             .remove_entry(&peer_entry.0.clone())
        //             .unwrap()
        //     }
        //     ShareName::Full(full_share_name) => self
        //         .remote_shares
        //         .remove_entry(&full_share_name)
        //         .ok_or(NoSuchRemoteShareError)?,
        // };
        //
        // let peer_id = remote_share.owner;
        // let peer = self.peers.get_mut(&peer_id).unwrap();
        // let res = peer.used_remote_shares.remove(&remote_share_name.name);
        // debug_assert!(res);
        // self.try_drop_peer(peer_id);
        // self.try_close_server();
        // Ok(())
    }

    /// Sends a global shutdown signal if server has no peers and no shares
    pub fn try_close_server(&self) {
        if self.peers.is_empty() && self.local_shares.is_empty() {
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
    pub address: SocketAddr,
    pub conn_notification: mpsc::Sender<PeerNotification>,
    kill_peer_conn: CancellationToken,
    /// Shares of the peer that we are using
    used_remote_shares: BTreeSet<ShareName>,
    /// Our shares that the peer is using
    used_shares: BTreeSet<ShareName>,
}

impl Peer {
    fn new(address: SocketAddr, notification_tx: mpsc::Sender<PeerNotification>) -> Self {
        Self {
            address,
            conn_notification: notification_tx,
            kill_peer_conn: SERVER_CANCEL.child_token(),
            used_remote_shares: Default::default(),
            used_shares: Default::default(),
        }
    }
}

#[derive(Encode, Decode, Clone, Debug)]
pub struct Share {
    pub name: ShareName,
    pub path: String,
    pub participants: BTreeSet<PeerId>,
}

impl Share {
    pub fn new(name: ShareName, path: String) -> Self {
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
    pub name: ShareName,
    pub mount_path: PathBuf,
}
