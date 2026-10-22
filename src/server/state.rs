use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    path::PathBuf,
    sync::Arc,
};

use bitcode::{Decode, Encode};
use derive_more::{Display, Error};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::{
    common::{
        PeersDto, RemoteSharesDto,
        shares::{ShareLocator, ShareName},
    },
    server::{SERVER_CANCEL, peer::PeerId},
};

pub type SharedState = Arc<RwLock<State>>;

pub struct State {
    peers: BTreeMap<PeerId, Peer>,
    remote_shares: BTreeMap<ShareLocator, RemoteShare>,
    local_shares: BTreeMap<ShareName, Share>,

    endpoint: quinn::Endpoint,
}

impl State {
    pub fn new(endpoint: quinn::Endpoint) -> Self {
        Self {
            peers: Default::default(),
            remote_shares: Default::default(),
            local_shares: Default::default(),
            endpoint,
        }
    }

    pub fn get_endpoint(&self) -> &quinn::Endpoint {
        &self.endpoint
    }

    pub fn get_share_names(&self) -> Vec<ShareName> {
        self.local_shares.keys().cloned().collect()
    }

    pub fn peers_dto(&self) -> PeersDto {
        todo!()
        // let mut data = BTreeMap::new();
        // for (peer_id, peer) in &self.peers {
        //     data.insert(peer_id.clone(), peer.address);
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

    pub fn shares(&self) -> &BTreeMap<ShareName, Share> {
        &self.local_shares
    }

    pub fn new_peer_connection(&mut self, mut peer_data: Peer) -> Result<(), StateError> {
        match self.peers.entry(peer_data.peer_id) {
            Entry::Vacant(entry) => {
                peer_data.peer_id = entry.key().to_owned();
                entry.insert(peer_data);
                Ok(())
            }
            Entry::Occupied(_) => Err(StateError::PeerAlreadyConnected),
        }
    }

    pub fn create_share(&mut self, mut share: Share) -> Result<(), StateError> {
        todo!()
        // let common_name = share.name;
        // let entry = self.local_shares.entry(common_name);
        // match entry {
        //     Entry::Vacant(entry) => {
        //         share.name = entry.key().clone();
        //         entry.insert(share);
        //         Ok(())
        //     }
        //     Entry::Occupied(_) => Err(RepeatedShare),
        // }
    }

    pub fn remove_share(&mut self, name: &ShareName) -> Result<(), StateError> {
        todo!()
    }

    pub fn exit_remote_share(&mut self, remote_share_name: ShareName) -> Result<(), StateError> {
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

#[derive(Encode, Decode, Clone, Debug, Display, Error)]
pub enum StateError {
    #[display("Established a second connection to a peer")]
    PeerAlreadyConnected,
}

#[derive(Clone, Debug)]
pub struct Peer {
    pub peer_id: PeerId,
    pub conn: quinn::Connection,
    pub cancel_token: CancellationToken,
    /// Shares of the peer that we are using
    used_remote_shares: BTreeSet<ShareName>,
    /// Our shares that the peer is using
    used_shares: BTreeSet<ShareName>,
}

impl Peer {
    pub fn new(conn: quinn::Connection, peer_id: PeerId) -> Self {
        Self {
            peer_id,
            conn,
            cancel_token: SERVER_CANCEL.child_token(),
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
