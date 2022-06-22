use serde::{Serialize, Deserialize};
use libp2p::{identity, Multiaddr};
use zeroize::Zeroize;
use std::{ error::Error, fmt };
use tokio::fs;
use base58::ToBase58;
pub use nyumt_proto::keys::{ PublicKey, LocalPeerKey };

use crate::NYUMT_DIR;

pub fn generate_keypair() -> LocalPeerKey {
    LocalPeerKey::ED25519(identity::ed25519::Keypair::generate())
}

pub async fn save_keypair(keys: &LocalPeerKey) -> Result<(), Box<dyn Error>> {
    let mut keyfile = NYUMT_DIR.clone();
    keyfile.push("keys.conf");
    
    let mut encoded_keys = match keys {
        LocalPeerKey::ED25519(k) => k.encode()
    };

    let file    = fs::write(match keyfile.to_str() {
        Some(v) => v,
        None    => {
            encoded_keys.zeroize();
            return Err( Box::new( std::io::Error::new( std::io::ErrorKind::Other, "Error getting keyfile name" ) ) )
        }
    }, &encoded_keys).await?;
    encoded_keys.zeroize();
    Ok(())
}

pub async fn get_keypair() -> Result<LocalPeerKey, Box<dyn Error>> {
    let mut keyfile = NYUMT_DIR.clone();
    keyfile.push("keys.conf");
    let mut keys = tokio::fs::read(match keyfile.to_str() {
        Some(v) => v,
        None    => {
            return Err( Box::new( std::io::Error::new( std::io::ErrorKind::Other, "Error getting keyfile name" ) ) )
        }
    }).await?;

    Ok(LocalPeerKey::ED25519(identity::ed25519::Keypair::decode(&mut keys)?))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PeerId {
    pub public: String,
    pub addr: Option<String>
}

pub fn parse_peerid(peer: &str) -> Result<PeerId, Box<dyn Error>> {
    let spl = peer.split(";").collect::<Vec<&str>>();
    if spl.len() > 2 {
        return Err( Box::new( std::io::Error::new( std::io::ErrorKind::Other, "Error parsing peer id" ) ) );
    }
    Ok(PeerId { public: spl[0].to_owned(), addr: if spl.len() == 2 {
        let addr: Multiaddr = spl[1].parse()?;
        Some(spl[1].to_owned())
    } else {
        None
    }})
}
