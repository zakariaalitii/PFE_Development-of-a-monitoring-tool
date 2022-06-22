use borsh::{BorshSerialize, BorshDeserialize};
use sha2::{Sha256, Digest};

use crate::stats;

#[derive(Debug, BorshSerialize, BorshDeserialize, serde::Serialize, serde::Deserialize)]
pub struct GlobalConfig {
    pub master: MasterConfig,
    pub node: NodeConfig
}

impl GlobalConfig {
    pub fn default() -> GlobalConfig {
        GlobalConfig {
            master: MasterConfig::default(),
            node: NodeConfig::default()
        }
    }

    pub fn update(&mut self) {
        self.master.update();
        self.node.update();
    }
}

#[derive(Debug, BorshSerialize, BorshDeserialize, Clone, serde::Serialize, serde::Deserialize)]
pub struct MasterConfig {
    /// Hash calculated everytime this struct is mutated
    #[borsh_skip]
    #[serde(skip_serializing, skip_deserializing)]
    pub hash: [u8; 32],

    /// Collect data from master node too
    pub collect_data_from_master: bool,
    /// Number of retries before droppping new block when other nodes are unreachable or block
    /// rejected
    pub block_push_retries: u8,
    /// Vote time for block in millisecs
    pub block_vote_time: u64,
    /// Vote retries when facing errors (network, ...)
    pub vote_push_retries: u8,
    /// Poll timeout for client events in seconds
    pub poll_timeout: u64,
}

impl MasterConfig {
    pub fn default() -> MasterConfig {
        let mut conf = MasterConfig {
            hash: [0u8; 32],

            collect_data_from_master: false,
            block_push_retries: 3,
            block_vote_time: 5000,
            vote_push_retries: 4,
            poll_timeout: 2
        };
        conf.update();
        conf
    }

    pub fn update(&mut self) {
        let mut hasher = Sha256::new();
        let ser    = match self.try_to_vec() {
            Ok(v)  => v,
            Err(e) => panic!("Config struct can't be serialized: {}", e)
        };
        hasher.update(&ser);
        self.hash = hasher.finalize().try_into().expect("Sha256 must be 32 bytes");
    }
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, serde::Serialize, serde::Deserialize)]
pub struct NodeConfig {
    /// Hash calculated everytime this struct is mutated
    #[borsh_skip]
    #[serde(skip_serializing, skip_deserializing)]
    pub hash: [u8; 32],

    /// Report timeframe in seconds
    pub report_timeframe: u64,
    // Verification level
    // 0: None,
    // 1: Verify from another node if available
    // 2: Verfiy from all master nodes
    //pub verify_level: u8,
    /// Data collection struct
    pub db_struct: stats::DbStruct
}

impl NodeConfig {
    #[inline]
    pub fn default() -> NodeConfig {
        let mut conf = NodeConfig {
            hash: [0u8; 32],
            report_timeframe: 5,
            //verify_level: 2,
            db_struct: stats::DbStruct::new()
        };
        conf.update();
        conf
    }
    
    pub fn update(&mut self) {
        let mut hasher = Sha256::new();
        let ser    = match self.try_to_vec() {
            Ok(v)  => v,
            Err(e) => panic!("Config struct can't be serialized: {}", e)
        };
        hasher.update(&ser);
        self.hash = hasher.finalize().try_into().expect("Sha256 must be 32 bytes");
    }
}
