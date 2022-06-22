use borsh::{BorshSerialize, BorshDeserialize};

use crate::{
    types::{ RequestType, RequestStatus, RequestError },
    keys::PublicKey,
    stats::{OID, Report}
};

type VERSION = f64;
type SIG     = Vec<u8>;

/// Generic response
#[repr(u8)]
#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub enum ReqStat {
    Good,
    Bad
}

#[repr(u16)]
#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub enum ClientRequest {
    /// Request on connect to verify there is no version mismatch
    Hello(VERSION, PublicKey, SessionSettings),
    /// Sending block
    Block(Vec<RequestBlock>),
    /// Sending stats
    Stats(Stats),
    /// New order coming
    Request(Request),
    /// Sending stat
    Stat(Stats),
    /// Get live stats of a node
    GetStats(PublicKey, OID)
}

#[repr(u16)]
#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub enum ClientResponse {
    /// Hello request response
    Hi(VERSION),
    /// We got blocks
    Block(ReqStat),
    /// We got stats
    Stats(ReqStat),
    /// Request result
    Request(RequestResp),
    /// Incompatible version, with minimum supported
    IncompatibleVersion(VERSION),
    /// Response of Getstats
    GetStats(RequestResp),
    /// Fallback response when request is unknewn
    Error
}

/// Get stats response
#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub enum GetStats {
    Stats(Report, SIG),
    Error(crate::types::RequestStatus)
}

/// New opened session at /hello
#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct Hello {
    pub name: String,
    pub last_access: u64
}


/// Creating a request at /request 
#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct Request {
    pub req: Vec<RequestType>,
    pub sign: Vec<u8>,
}

/// Request status or error
#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub enum RequestResp {
    /// Error occured while verifying legitimacy of request
    RequestError(RequestError),
    /// Block treatment status
    RequestStatus(RequestStatus)
}

/// Client session settings
#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct SessionSettings {
    /// Request height to start from
    pub last_request_height: i64,
    /// Last stats timestamp
    pub last_timestamp: u64
}

/// Request block
#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct RequestBlock {
    pub id: u64,
    pub body: Vec<u8>
}

#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct Stat {
    /// Node id
    pub id: u32,
    /// Report
    pub data: Vec<u8>,
    /// Signature
    pub sig: Vec<u8>
}

#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct Stats(pub Vec<Stat>);
