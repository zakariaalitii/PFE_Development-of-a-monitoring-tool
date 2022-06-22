use libp2p::{identity, Multiaddr};
use base58::FromBase58;
use tracing::error;

use crate::utils::keys::PeerId;
use crate::database::{Database, node};

pub async fn multi_master(db: &Database) -> bool {
    match node::master_node_count(&db).await {
        Ok(v) => if v > 1 { true } else { false },
        _     => false
    }
}

/// Parse all peers info
/// Any error will be skipped and logged
pub async fn knewn_peers(peers: &mut Vec<PeerId>) -> Vec<(identity::PublicKey, Option<Multiaddr>, u32)> {
    let mut p = Vec::new();
    let mut peer;
    while !peers.is_empty() {
        peer = peers.pop().unwrap();
        p.push((identity::PublicKey::Ed25519(
        match identity::ed25519::PublicKey::decode(&match peer.public.from_base58() {
            Ok(v)  => v,
            Err(_) => {
                error!("Error converting peer publickey {} from base58", peer.public);
                continue;
            }
        }) {
            Ok(v)  => v,
            Err(_) => {
                error!("Error decoding peer publickey {}", peer.public);
                continue;
            }
        }),
        if peer.addr.is_none() {
            None
        } else {
            let addr = peer.addr.unwrap();
            Some(match addr.parse::<Multiaddr>() {
                Ok(v)  => v,
                Err(_) => {
                    error!("Error parsing peer {} address {}", peer.public, addr);
                    continue;
                }
            })
        },
        1));
    }
    p
}

#[inline(always)]
pub fn peer_knewn<'a>(knewn_peers: &'a Vec<(identity::PublicKey, Option<Multiaddr>, u32)>, new_peer: &libp2p::PeerId) -> Option<&'a (identity::PublicKey, Option<Multiaddr>, u32)> {
    for peer in knewn_peers {
        return match new_peer.is_public_key(&peer.0) {
            Some(true) => Some(&peer),
            _          => None
        };
    }
    None
}
