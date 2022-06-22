use std::{
    error::Error,
    fmt
};
use borsh::{ BorshSerialize, BorshDeserialize };
use libp2p::identity;
use base58::{ ToBase58, FromBase58 };
use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    static ref STRING_ED25519: Regex = Regex::new(r"ed25519\((.*?)\)").unwrap();
}

#[repr(u8)]
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum PublicKey {
    ED25519([u8; 32])
}

impl PublicKey {
    pub fn parse(pubkey: &[u8]) -> Result<PublicKey, Box<dyn Error>> {
        if pubkey.len() == 32 {
            match pubkey.to_owned().try_into() {
                Ok(v)  => return Ok(PublicKey::ED25519(v)),
                Err(_) => ()
            }
        }
        Err( Box::new( std::io::Error::new( std::io::ErrorKind::Other, "Error parsing public key" ) ) )
    }

    pub fn parse_str(pubkey: &str) -> Result<PublicKey, Box<dyn Error>> {
        let cap = STRING_ED25519.captures(pubkey);
        match cap {
            Some(caps) => {
                let key = caps.get(1);
                if key.is_some() {
                    return PublicKey::from_base58(key.unwrap().as_str());
                }
            },
            None => ()
        }
        Err( Box::new( std::io::Error::new( std::io::ErrorKind::Other, "Error parsing public key" ) ) )
    }

    pub fn to_publickey(&self) -> Result<identity::PublicKey, Box<dyn Error + Send + Sync>> {
        match self {
            PublicKey::ED25519(v) => Ok(identity::PublicKey::Ed25519(identity::ed25519::PublicKey::decode(v)?))
        }
    }
    
    pub fn to_peer_id(&self) -> Result<libp2p::PeerId, Box<dyn Error + Send + Sync>> {
        match self {
            PublicKey::ED25519(v) => Ok(identity::PublicKey::Ed25519(identity::ed25519::PublicKey::decode(v)?).to_peer_id())
        }
    }

    pub fn to_base58(&self) -> String {
        match self {
            PublicKey::ED25519(v) => v.to_base58()
        }
    }

    pub fn from_base58(str: &str) -> Result<PublicKey, Box<dyn Error>> {
        Ok(PublicKey::ED25519(match str.from_base58() {
            Ok(v)  => match v.try_into() {
                Ok(v)  => v,
                Err(_) => return Err( Box::new( std::io::Error::new( std::io::ErrorKind::Other, "Wrong public key size" ) ) )
            },
            Err(_) => return Err( Box::new( std::io::Error::new( std::io::ErrorKind::Other, "Error parsing public key" ) ) )
        }))
    }

    pub fn verify_sig(&self, msg: &[u8], sig: &[u8]) -> Result<bool, Box<dyn Error + Send + Sync>> {
        match self {
            PublicKey::ED25519(v) => {
                let key = identity::PublicKey::Ed25519(identity::ed25519::PublicKey::decode(v)?);
                Ok(key.verify(msg, sig))
            }
        }
    }

    pub fn raw(&self) -> Vec<u8> {
        match self {
            PublicKey::ED25519(v) => v.to_vec(),
        }
    }

    #[inline]
    pub fn export(&self) -> String {
        match self {
            PublicKey::ED25519(_) => format!("ed25519({})", self.to_base58())
        }
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.export())
    }
} 

#[derive(Clone)]
pub enum LocalPeerKey {
    ED25519(identity::ed25519::Keypair)
}

impl LocalPeerKey {
    pub fn generate() -> LocalPeerKey {
        LocalPeerKey::ED25519(identity::ed25519::Keypair::generate())
    }
    pub fn public(&self) -> PublicKey {
        match &self {
            LocalPeerKey::ED25519(v) => PublicKey::ED25519(v.public().encode())
        }
    }
}

impl fmt::Display for LocalPeerKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.public())
    }
}
