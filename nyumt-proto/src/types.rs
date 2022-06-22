use borsh::{ BorshSerialize, BorshDeserialize };

use crate::keys::PublicKey;
use crate::config::{MasterConfig, NodeConfig};
use crate::stats::Report;

#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub struct Block {
    pub hash: [u8; 32],
    pub body: Vec<u8> 
}

#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub struct BodyData {
    pub req: Vec<RequestType>,
    pub sign: Vec<u8>,
    pub user_pk: PublicKey,
    pub timestamp: u64,
}

#[repr(u8)]
#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub enum RequestType {
    Order(Order)
}

#[repr(u16)]
#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub enum Order {
    AddUser { name: String, publickey: PublicKey, perm: Permissions },
    AddMasterNode { name: String, publickey: PublicKey },
    AddNode { name: String, publickey: PublicKey },

    EditUser { name: String, publickey: PublicKey, perm: Permissions },
    EditMasterNode { name: String, publickey: PublicKey },
    EditNode { name: String, publickey: PublicKey },

    RemoveUser { publickey: PublicKey },
    RemoveMasterNode { publickey: PublicKey },
    RemoveNode { publickey: PublicKey },

    ChangeNodeConfig { publickey: PublicKey, name: String },

    UpdatePublicKey { publickey: PublicKey, new_publickey: PublicKey },
    
    SetConfig { name: String, master: MasterConfig, node: NodeConfig },
    RemoveConfig { name: String },

    UpdateVoteWeight { publickey: PublicKey, weight: u32 }
}

#[repr(u8)]
#[derive(Clone, BorshSerialize, BorshDeserialize, Debug, PartialEq)]
pub enum Permission {
    /// Access to anything
    Root = 0,
    // Access to logs
    //Log = 1,
    /// Access to data analytics
    Monitor = 2,
    // File uploads
    //Repo = 3,
}

#[derive(Clone, BorshSerialize, BorshDeserialize, Debug)]
pub struct Permissions(pub Vec<Permission>);

impl std::fmt::Display for Permissions {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[repr(u8)]
#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub enum RequestError {
    /// Error with signature
    SignatureError,
    /// Parameter too long
    LongParameter,
    /// Permission denied
    PermissionError,
    /// Empty permissions (user need one permission at least)
    NoPermission,
    /// Entity with publickey doesn't exist
    InvalidEntity,
    /// Duplicate entity, happens when adding duplicate user/node
    DuplicateEntity,
    /// Config currently used by a node
    ConfigUsed,
    /// Config not found
    InvalidConfig
}

#[repr(u8)]
#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub enum RequestStatus {
    /// All good
    Ok,
    /// When first block is not present yet
    NotConfigured,
    /// Node still syncing after boot
    Syncing,
    /// Error handling public key
    PubKeyError,
    /// General error with request
    ReqError,
    /// General node failure
    NodeError,
    /// Duplicate request
    DupReq,
    /// Requested node inactive
    NodeInactive,
    /// Timeout reached
    RequestTimeout,
    /// Get stats response
    GetStats(Report, Vec<u8>),
    /// Rejected for lack of vote or there is no quorum
    Rejected
}

#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct NodeReport {
    pub report: Vec<u8>,
    pub sig: Vec<u8>
}
