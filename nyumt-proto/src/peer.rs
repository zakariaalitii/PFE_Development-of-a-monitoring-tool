use borsh::{BorshSerialize, BorshDeserialize};
use sha2::{Sha256, Digest};
use std::error::Error;

use crate::keys::PublicKey;
use crate::types::NodeReport;
use crate::config::NodeConfig;
use crate::stats::{OID, Report};

#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub struct MasterNode {
    pub pubkey: PublicKey,
    pub addr: Option<String>
}

#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub struct MasterNodes(pub Vec<MasterNode>);

impl MasterNodes {
    pub fn hash(&self) -> Result<[u8; 32], Box<dyn Error>> {
        let mut hash = Sha256::new();
        for n in &self.0 {
            hash.update(n.pubkey.try_to_vec()?);
        }
        Ok(hash.finalize().into())
    }
}

type VERSION = f64;
type HEIGHT  = u64;
type HASH    = [u8; 32];
type MasterNodeList = MasterNodes;
type SIG     = Vec<u8>;

/// Generic response
#[repr(u8)]
#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub enum RequestStatus {
    Good,
    Bad
}


#[repr(u16)]
#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub enum PeerRequest {
    /// Request on connect to verify there is no version mismatch
    Hello(VERSION, PublicKey),
    /// Hello with subscribtion
    HelloSubscribe,
    /// Get last block height
    BlockHeight,
    /// Node config
    NodeConfig,
    /// Node config hash
    NodeConfigHash,
    /// Master nodes list
    MasterNodeList,
    /// Master nodes list hash
    MasterNodeListHash,
    /// Unsubscribe default master node
    UnsubscribeDefaultNode,
    /// Update config
    UpdateConfig(NodeConfig),
    /// Update master node list
    UpdateMasterNodeList(MasterNodes),
    /// A node's report
    Report(NodeReport),
    /// Get live stats from a node
    GetStats(OID)
}

#[repr(u16)]
#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub enum PeerResponse {
    /// Hello request response
    Hi(VERSION),
    /// Block height response
    BlockHeight(HEIGHT),
    /// Node config response
    NodeConfig(NodeConfig),
    /// Node config hash response
    NodeConfigHash(HASH),
    /// Master node list
    MasterNodeList(MasterNodeList),
    /// Master node list hash
    MasterNodeListHash(HASH),
    /// Response to report
    Report(RequestStatus),
    /// Get stats response
    GetStats(Report, SIG),
    /// Generic Ok
    Good,
    /// Generic error
    Error
}
