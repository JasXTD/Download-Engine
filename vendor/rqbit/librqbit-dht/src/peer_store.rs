// Modified by Download Engine: replaced rand and dashmap. See ../../patch.md.
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    str::FromStr,
    sync::atomic::AtomicU32,
};

use bencode::ByteBufOwned;
use chrono::{DateTime, Utc};
use librqbit_core::{compact_ip::CompactSocketAddr, hash_id::Id20, random};
use parking_lot::RwLock;
use serde::{
    Deserialize, Serialize,
    ser::{SerializeMap, SerializeStruct},
};
use tracing::trace;

use crate::bprotocol::{AnnouncePeer, Want};

#[derive(Serialize, Deserialize)]
struct StoredToken {
    token: [u8; 4],
    #[serde(serialize_with = "crate::utils::serialize_id20")]
    node_id: Id20,
    addr: SocketAddr,
}

#[derive(Serialize, Deserialize)]
struct StoredPeer {
    addr: SocketAddr,
    time: DateTime<Utc>,
}

pub struct PeerStore {
    self_id: Id20,
    max_remembered_tokens: u32,
    max_remembered_peers: u32,
    max_distance: Id20,
    tokens: RwLock<VecDeque<StoredToken>>,
    peers: RwLock<HashMap<Id20, Vec<StoredPeer>>>,
    peers_len: AtomicU32,
}

impl Serialize for PeerStore {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        struct SerializePeers<'a> {
            peers: &'a RwLock<HashMap<Id20, Vec<StoredPeer>>>,
        }

        impl Serialize for SerializePeers<'_> {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                let mut m = serializer.serialize_map(None)?;
                for (info_hash, peers) in self.peers.read().iter() {
                    m.serialize_entry(&info_hash.as_string(), peers)?;
                }
                m.end()
            }
        }

        let mut s = serializer.serialize_struct("PeerStore", 7)?;
        s.serialize_field("self_id", &self.self_id.as_string())?;
        s.serialize_field("max_remembered_tokens", &self.max_remembered_tokens)?;
        s.serialize_field("max_remembered_peers", &self.max_remembered_peers)?;
        s.serialize_field("max_distance", &self.max_distance.as_string())?;
        s.serialize_field("tokens", &*self.tokens.read())?;
        s.serialize_field("peers", &SerializePeers { peers: &self.peers })?;
        s.serialize_field(
            "peers_len",
            &self.peers_len.load(std::sync::atomic::Ordering::SeqCst),
        )?;
        s.end()
    }
}

impl<'de> Deserialize<'de> for PeerStore {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Tmp {
            self_id: Id20,
            max_remembered_tokens: u32,
            max_remembered_peers: u32,
            max_distance: Id20,
            tokens: VecDeque<StoredToken>,
            peers: HashMap<Id20, Vec<StoredPeer>>,
        }

        Tmp::deserialize(deserializer).map(|tmp| Self {
            self_id: tmp.self_id,
            max_remembered_tokens: tmp.max_remembered_tokens,
            max_remembered_peers: tmp.max_remembered_peers,
            max_distance: tmp.max_distance,
            tokens: RwLock::new(tmp.tokens),
            peers_len: AtomicU32::new(tmp.peers.values().map(|v| v.len() as u32).sum()),
            peers: RwLock::new(tmp.peers),
        })
    }
}

impl PeerStore {
    pub fn new(self_id: Id20) -> Self {
        Self {
            self_id,
            max_remembered_tokens: 1000,
            max_remembered_peers: 1000,
            max_distance: Id20::from_str("00000fffffffffffffffffffffffffffffffffff").unwrap(),
            tokens: RwLock::new(VecDeque::new()),
            peers: RwLock::new(HashMap::new()),
            peers_len: AtomicU32::new(0),
        }
    }

    pub fn gen_token_for(&self, node_id: Id20, addr: SocketAddr) -> [u8; 4] {
        let mut token = [0u8; 4];
        random::fill(&mut token);
        let mut tokens = self.tokens.write();
        tokens.push_back(StoredToken {
            token,
            addr,
            node_id,
        });
        if tokens.len() > self.max_remembered_tokens as usize {
            tokens.pop_front();
        }
        token
    }

    pub fn store_peer(&self, announce: &AnnouncePeer<ByteBufOwned>, mut addr: SocketAddr) -> bool {
        // If the info_hash in announce is too far away from us, don't store it.
        // If the token doesn't match, don't store it.
        // If we are out of capacity, don't store it.
        // Otherwise, store it.
        if announce.info_hash.distance(&self.self_id) > self.max_distance {
            trace!("peer store: info_hash too far to store");
            return false;
        }
        if !self.tokens.read().iter().any(|t| {
            t.token[..] == announce.token.as_ref()[..] && t.addr == addr && t.node_id == announce.id
        }) {
            trace!("peer store: can't find this token / addr combination");
            return false;
        }

        if announce.implied_port == 0 {
            addr.set_port(announce.port);
        }

        use std::collections::hash_map::Entry;
        let mut peers = self.peers.write();
        let peers_len = self.peers_len.load(std::sync::atomic::Ordering::SeqCst);
        match peers.entry(announce.info_hash) {
            Entry::Occupied(mut occ) => {
                if let Some(s) = occ.get_mut().iter_mut().find(|s| s.addr == addr) {
                    s.time = Utc::now();
                    return true;
                }
                if peers_len >= self.max_remembered_peers {
                    trace!("peer store: out of capacity");
                    return false;
                }
                occ.get_mut().push(StoredPeer {
                    addr,
                    time: Utc::now(),
                });
            }
            Entry::Vacant(vac) => {
                if peers_len >= self.max_remembered_peers {
                    trace!("peer store: out of capacity");
                    return false;
                }
                vac.insert(vec![StoredPeer {
                    addr,
                    time: Utc::now(),
                }]);
            }
        }

        self.peers_len
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        true
    }

    pub fn get_for_info_hash(&self, info_hash: Id20, want: Want) -> Vec<CompactSocketAddr> {
        if let Some(stored_peers) = self.peers.read().get(&info_hash) {
            return stored_peers
                .iter()
                .filter(|p| {
                    matches!(
                        (p.addr, want),
                        (SocketAddr::V6(..), Want::V6 | Want::Both)
                            | (SocketAddr::V4(..), Want::V4 | Want::Both)
                    )
                })
                .map(|p| p.addr.into())
                .collect();
        }
        Vec::new()
    }

    #[allow(dead_code)]
    pub fn garbage_collect_peers(&self) {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering::SeqCst;

    /// Close enough to a store with an all-zero id to be accepted.
    fn near_hash(last: u8) -> Id20 {
        let mut bytes = [0u8; 20];
        bytes[19] = last;
        Id20::new(bytes)
    }

    fn peer(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    fn announce_with(token: &[u8], info_hash: Id20, port: u16) -> AnnouncePeer<ByteBufOwned> {
        AnnouncePeer {
            id: near_hash(1),
            implied_port: 1,
            info_hash,
            port,
            token: token.into(),
        }
    }

    fn announce(store: &PeerStore, info_hash: Id20, addr: SocketAddr) -> bool {
        let token = store.gen_token_for(near_hash(1), addr);
        store.store_peer(&announce_with(&token, info_hash, 0), addr)
    }

    fn stored(store: &PeerStore, info_hash: Id20) -> Vec<SocketAddr> {
        let mut addrs: Vec<SocketAddr> = store
            .get_for_info_hash(info_hash, Want::Both)
            .into_iter()
            .map(|p| p.0)
            .collect();
        addrs.sort();
        addrs
    }

    #[test]
    fn test_stored_peers_come_back_for_their_torrent() {
        let store = PeerStore::new(Id20::default());
        assert!(announce(&store, near_hash(5), peer(1)));
        assert!(announce(&store, near_hash(5), peer(2)));
        assert!(announce(&store, near_hash(6), peer(3)));
        assert_eq!(stored(&store, near_hash(5)), [peer(1), peer(2)]);
        assert_eq!(stored(&store, near_hash(6)), [peer(3)]);
        assert_eq!(stored(&store, near_hash(7)), []);
    }

    #[test]
    fn test_announced_port_is_used_unless_implied() {
        let store = PeerStore::new(Id20::default());
        let token = store.gen_token_for(near_hash(1), peer(1));
        let mut message = announce_with(&token, near_hash(5), 6881);
        message.implied_port = 0;
        assert!(store.store_peer(&message, peer(1)));
        assert_eq!(stored(&store, near_hash(5)), [peer(6881)]);
    }

    #[test]
    fn test_wrong_token_is_refused() {
        let store = PeerStore::new(Id20::default());
        let token = store.gen_token_for(near_hash(1), peer(1));
        let mut wrong = token;
        wrong[0] ^= 1;
        assert!(!store.store_peer(&announce_with(&wrong, near_hash(5), 0), peer(1)));
        // A token only works from the address it was given to.
        assert!(!store.store_peer(&announce_with(&token, near_hash(5), 0), peer(2)));
        assert_eq!(stored(&store, near_hash(5)), []);
    }

    #[test]
    fn test_torrent_far_from_our_id_is_refused() {
        let store = PeerStore::new(Id20::default());
        assert!(!announce(&store, Id20::new([0xff; 20]), peer(1)));
    }

    #[test]
    fn test_same_peer_is_stored_once() {
        let store = PeerStore::new(Id20::default());
        assert!(announce(&store, near_hash(5), peer(1)));
        assert!(announce(&store, near_hash(5), peer(1)));
        assert_eq!(stored(&store, near_hash(5)), [peer(1)]);
        assert_eq!(store.peers_len.load(SeqCst), 1);
    }

    #[test]
    fn test_full_store_refuses_new_peers_but_keeps_known_ones() {
        let mut store = PeerStore::new(Id20::default());
        store.max_remembered_peers = 2;
        assert!(announce(&store, near_hash(5), peer(1)));
        assert!(announce(&store, near_hash(6), peer(2)));
        assert!(!announce(&store, near_hash(5), peer(3)));
        assert!(!announce(&store, near_hash(7), peer(3)));
        assert!(announce(&store, near_hash(5), peer(1)));
        assert_eq!(store.peers_len.load(SeqCst), 2);
    }

    #[test]
    fn test_saved_store_loads_with_the_same_peers() {
        let store = PeerStore::new(Id20::default());
        assert!(announce(&store, near_hash(5), peer(1)));
        assert!(announce(&store, near_hash(5), peer(2)));
        assert!(announce(&store, near_hash(6), peer(3)));
        let json = serde_json::to_string(&store).unwrap();
        let loaded: PeerStore = serde_json::from_str(&json).unwrap();
        assert_eq!(stored(&loaded, near_hash(5)), [peer(1), peer(2)]);
        assert_eq!(stored(&loaded, near_hash(6)), [peer(3)]);
        assert_eq!(loaded.peers_len.load(SeqCst), 3);
    }
}
