use tokio::sync::oneshot;
use borsh::{BorshSerialize, BorshDeserialize};
use libp2p::PeerId;

pub use nyumt_proto::types::{ BodyData, RequestType, Order, Permission, Permissions, RequestStatus, Block };
use nyumt_proto::stats::OID;

#[derive(Debug)]
pub enum ChannelRequestType {
    Block(BodyData),
    GetStats(PeerId, OID)
}

pub struct Request {
    pub body: ChannelRequestType,
    // To get response to request we create a oneshot channel
    // Response will be a status code
    pub resp_channel: oneshot::Sender<RequestStatus>
}

pub enum NetEvent {
    NewBlock {
        body: BodyData,
        hash: [u8; 32]
    },
    ArchiveBlock(ArchiveBlock),
    Vote([u8; 32], Vote, u32),
    SerialVote(u64, [u8; 32], Vote, u32),
    PendingBlockTimeout([u8; 32]),
    ApprovedBlockTimeout(u64, [u8; 32], [u8; 32])
}

#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub struct ArchiveBlock {
    /// Block raw hash
    pub hash: [u8; 32],
    /// Block serial hash (old block hash digested with block)
    pub serial_hash: [u8; 32],
    pub id: u64
}

#[repr(u8)]
#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub enum Vote {
    Ok,
    No,
}
