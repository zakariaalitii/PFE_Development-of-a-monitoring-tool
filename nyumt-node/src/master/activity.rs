use libp2p::{
    Swarm,
    gossipsub::{IdentTopic as Topic, error::PublishError, MessageId },
    PeerId,
    Multiaddr, request_response::{ResponseChannel, RequestId}
};
use std::{
    error::Error,
    sync::Arc,
    collections::HashMap
};
use tokio::{
    time::{timeout, Duration},
    sync::{
        RwLock,
        mpsc::{Receiver, Sender},
        oneshot
    }
};
use tracing::{warn, info, error};
use sha2::{Sha256, Digest};
use rand::{SeedableRng, Rng};
use borsh::{BorshSerialize, BorshDeserialize};
use futures::StreamExt;

use nyumt_proto::{
    db::{self as db_interpret, Update},
    peer::{PeerRequest, MasterNode, MasterNodes, PeerResponse},
    database::{cache, node},
    config::{MasterConfig, NodeConfig},
    stats::{OID, System, Platform, DbStruct, generate_report}
};

use crate::master::{
    net_behavior::{ NyumtProt, NetData, NyumtReq },
    types::{ Request, RequestStatus, Block, BodyData, ArchiveBlock, NetEvent, Vote },
    SwarmSettings,
    TotalVote
};
use crate::database::block;
use crate::POLL_TIME;
use super::{peer, types::ChannelRequestType};
use crate::utils::keys::PublicKey;

pub async fn get_request(recv: &mut Receiver<Request>) -> Result<Request, Box<dyn Error + Send + Sync>> {
    match recv.recv().await {
        Some(v) => Ok(v),
        None => {
            error!("Request channel closed, exiting");
            return Err( Box::new( std::io::Error::new( std::io::ErrorKind::Other, "Error while reading requests" ) ) );
        }
    }
}

pub fn serialize_req(block: &BodyData) -> Result<(Vec<u8>, [u8; 32]), Box<dyn Error>> {
    let mut hasher     = Sha256::new();
    let bytes          = block.try_to_vec()?;

    hasher.update(bytes.as_slice());

    let hash: [u8; 32] = hasher.finalize().try_into().expect("Sha256 returning wrong length");
    let block = NetData::Block(Block {
        hash: hash.clone(),
        body: bytes.to_vec()
    });

    let bytes = block.try_to_vec()?;

    Ok((bytes.to_vec(), hash))
} 

#[inline(always)]
pub fn serialize_block(block: &BodyData, vhash: Option<[u8; 32]>, hasherfn: Option<Sha256>) -> Result<(Vec<u8>, [u8; 32]), Box<dyn Error>> {
    let hash;
    let bytes = block.try_to_vec()?;
    match vhash {
        Some(v) => hash = v,
        None    => {
            let mut hasher     = if hasherfn.is_none() { Sha256::new() } else { hasherfn.unwrap() };
            let bytes          = block.try_to_vec()?;

            hasher.update(bytes.as_slice());

            hash = hasher.finalize().try_into().expect("Sha256 returning wrong length");
        }
    }

    let block = Block {
        hash: hash.clone(),
        body: bytes.to_vec()
    };

    Ok((block.try_to_vec()?, hash))
}

pub enum BlockPublishError {
    /// General failure, caused by some limits (allocation)
    GeneralFailure,
    /// Something wrong with request
    ReqError,
    /// Duplicate request
    DupReq,
    /// Rejected request
    Rejected
}

#[inline(always)]
async fn block_order_vote(swarm: &mut SwarmHandlerCall, swarm_set: &Arc<SwarmSettings>,
                              topic: &Topic, bytes: &[u8]) -> Result<(u64, [u8; 32]), BlockPublishError> {
    let mut new_block = Err(BlockPublishError::ReqError);
    let mut retry     = 0u8;
    let archived      = NetData::try_from_slice(&bytes[..]);
    let data: Block   = match archived {
        Ok(NetData::Block(v)) => v,
        _     => return Err(BlockPublishError::GeneralFailure)
    };
    'APPROVE_VOTE: loop {
        let (height, hash) = match block::get_last_block(&swarm_set.db.clone()).await {
            Ok(Some(v)) => v,
            _           => {
                error!("Error reading from database. database may have been externally modified while node running.");
                return Err(BlockPublishError::Rejected);
            }
        };
        let mut hasher = Sha256::new();
        hasher.update(hash.as_slice());
        hasher.update(data.body.as_slice());
        let finhash: [u8; 32] = hasher.finalize().try_into().expect("Sha256 must be 32 bytes");
        let block_height      = height + 1;
        let ser_vote = NetData::ArchiveBlock(ArchiveBlock { hash: data.hash.clone(), serial_hash: finhash.clone(), id: block_height });
        let bytes = match ser_vote.try_to_vec() {
            Ok(v)   => v,
            Err(_)  => return Err(BlockPublishError::GeneralFailure)
        };

        match swarm.publish(topic.to_owned(), bytes.to_owned()).await {
            Ok(_) => {
                {
                    let vote_weight  = swarm_set.vote;
                    swarm_set.approved_blocks.write().await.push((block_height, finhash.clone(), TotalVote { ok: vote_weight, no: 0 }, 1));
                }
                await_votes(swarm, swarm_set, &finhash, false).await;
                {
                    for pending in swarm_set.approved_blocks.read().await.as_slice() {
                        if pending.1 == finhash {
                            // We need to always have majority vote
                            if pending.2.no > pending.2.ok {
                                if retry >= swarm_set.conf.read().await.master.block_push_retries {
                                    // No agreement reached
                                    new_block = Err(BlockPublishError::Rejected);
                                } else {
                                    retry += 1;
                                    let mut seed: rand::rngs::StdRng = SeedableRng::from_entropy();
                                    let rand: u64 = seed.gen_range(500..=4000);
                                    tokio::time::sleep(Duration::from_millis(rand)).await;
                                    continue 'APPROVE_VOTE;
                                }
                            } else {
                                new_block = Ok((block_height, finhash));
                            }
                            break;
                        }
                    }
                }
                swarm_set.approved_blocks.write().await.retain(|x| x.0 != block_height);
                return new_block;
            }
            Err(PublishError::InsufficientPeers) => {
                if retry >= swarm_set.conf.read().await.master.block_push_retries {
                    warn!("Other nodes are unreachable, block dropped");
                    return Err(BlockPublishError::ReqError);
                }
                retry += 1;
                tokio::time::sleep(*POLL_TIME).await;
            },
            Err(e) => {
                error!("Block skipped for {}", e);
                return Err(BlockPublishError::ReqError);
            },
        }
    }
}

async fn await_votes_tx(swarm: &mut SwarmHandlerCall, swarm_set: &Arc<SwarmSettings>, hash: [u8; 32], approved: Option<(u64, [u8; 32])>) {
    await_votes(swarm, swarm_set, &hash, approved.is_none()).await;
    if approved.is_none() {
        swarm_set.channel.clone().send(NetEvent::PendingBlockTimeout(hash)).await.ok();
    } else {
        let appr = approved.unwrap();
        swarm_set.channel.clone().send(NetEvent::ApprovedBlockTimeout(appr.0, hash, appr.1)).await.ok();
    }
}

async fn await_votes(swarm: &mut SwarmHandlerCall, swarm_set: &Arc<SwarmSettings>, hash: &[u8; 32], pending: bool) {
    match timeout(Duration::from_secs(swarm_set.conf.read().await.master.block_vote_time), async {
        loop {
            let connected_count = 1 + swarm.peers().await;
            {
                if pending {
                    let pending_blocks = swarm_set.pending_blocks.read().await;
                    for pending in pending_blocks.as_slice() {
                        if pending.0 == *hash && pending.2 == connected_count {
                            break;
                        }
                    }
                } else {
                    let approved_blocks = swarm_set.approved_blocks.read().await;
                    for pending in approved_blocks.as_slice() {
                        if pending.1 == *hash && pending.3 == connected_count {
                            break;
                        }
                    }
                }
            }
            tokio::time::sleep(*POLL_TIME).await;
        };
    }).await {
        Ok(_)   => (),
        Err(_)  => ()
    }
}

#[inline(always)]
async fn publish(swarm: &mut SwarmHandlerCall, swarm_set: &Arc<SwarmSettings>,
                     topic: &Topic, bytes: &[u8], id: u64, hash: &[u8; 32]) -> Result<Option<(u64, [u8; 32])>, BlockPublishError> {
    let mut retry = 0u8;
    loop {
       match swarm.publish(topic.to_owned(), bytes.to_owned()).await {
            Ok(_) => {
                swarm_set.pending_blocks.write().await.push((hash.clone(), TotalVote { ok: swarm_set.vote, no: 0 }, 1));
                // Now we wait for votes
                // We will not wait if a node never votes
                await_votes(swarm, swarm_set, hash, true).await;

                {
                    let pending_blocks = swarm_set.pending_blocks.read().await;
                    for pending in pending_blocks.as_slice() {
                        if pending.0 == *hash {
                            // We need to always have majority vote
                            if pending.1.no > pending.1.ok {
                                return Err(BlockPublishError::Rejected);
                            }
                        }
                    }
                }
                swarm_set.pending_blocks.write().await.retain(|x| x.0 != *hash);

                // Time to vote for block position
                match block_order_vote(swarm, swarm_set, topic, bytes).await {
                    Ok(v)  => return Ok(Some(v)),
                    Err(e) => return Err(e)
                }
            }, 
            Err(PublishError::InsufficientPeers) => {
                // In case there is peers we keep retrying until we reach them for x times
                if !peer::multi_master(&swarm_set.db.clone()).await {
                    break;
                } else if retry >= swarm_set.conf.read().await.master.block_push_retries {
                    warn!("Other nodes are unreachable, block dropped");
                    return Err(BlockPublishError::ReqError);
                }
                retry += 1;
                tokio::time::sleep(*POLL_TIME).await;
            },
            Err(e) => {
                error!("Block skipped for {}", e);
                return Err(BlockPublishError::ReqError);
            },
        }
    }
    Ok(None)
}

async fn handle_incoming_data(mut swarm: SwarmHandlerCall, swarm_set: Arc<SwarmSettings>, mut rx: Receiver<NetEvent>, tx: Sender<NetEvent>) {
    let mut approved_blocks = HashMap::new();
    loop {
        match rx.recv().await {
            Some(event) => {
                match event {
                    NetEvent::Vote(hash, vote, vote_weight) => {
                        let mut pending_blocks = swarm_set.pending_blocks.write().await;
                        for pending in pending_blocks.as_mut_slice() {
                            if pending.0 == hash {
                                match vote {
                                    Vote::Ok => pending.1.ok += vote_weight, 
                                    Vote::No => pending.1.no += vote_weight
                                }
                                pending.2 += 1;
                            }
                        }
                    },
                    NetEvent::SerialVote(id, hash, vote, vote_weight) => {
                        let mut approved_blocks_guard = swarm_set.approved_blocks.write().await;
                        for appr in approved_blocks_guard.as_mut_slice() {
                            if appr.0 == id && appr.1 == hash {
                                match vote {
                                    Vote::Ok  => appr.2.ok  += vote_weight,
                                    Vote::No  => appr.2.no  += vote_weight
                                }
                                appr.3 += 1;
                            }
                        }
                    },
                    NetEvent::NewBlock { body, hash } => {
                        let mut s          = swarm.clone();
                        let hash_clone     = hash.clone();
                        let swrm_set_clone = swarm_set.clone();
                        tokio::spawn(async move {
                            await_votes_tx(&mut s, &swrm_set_clone, hash_clone, None).await;
                        });
                        approved_blocks.insert(hash, match body.try_to_vec() {
                            Ok(v) => (v, body),
                            Err(e) => {
                                error!("Error serializing block: {}", e);
                                continue;
                            }
                        });
                    },
                    NetEvent::ArchiveBlock(block_info) => {
                        if approved_blocks.contains_key(&block_info.hash) {
                            let (height, hash) = match block::get_last_block(&swarm_set.db.clone()).await {
                                Ok(Some(v)) => v,
                                _           => {
                                    error!("Error reading from database. database may have been externally modified while node running.");
                                    continue;
                                }
                            };
                            let mut hasher = Sha256::new();
                            hasher.update(hash.as_slice());
                            hasher.update(approved_blocks.get(&block_info.hash).unwrap().0.as_slice());
                            let finhash: [u8; 32] = hasher.finalize().try_into().expect("Sha256 must be 32 bytes");
                            let block_height      = height + 1;
                            if finhash == block_info.serial_hash && block_height == block_info.id {
                                let topic          = swarm_set.topic.clone();
                                let mut s          = swarm.clone();
                                let hash_clone     = finhash.clone();
                                let swrm_set_clone = swarm_set.clone();
                                tokio::spawn(async move {
                                    await_votes_tx(&mut s, &swrm_set_clone, hash_clone, Some((block_height, block_info.hash))).await;
                                });
                                match swarm.publish(topic, match NetData::SerialVote(block_height, finhash, Vote::Ok).try_to_vec() {
                                    Ok(v)  => v,
                                    Err(e) => {
                                        error!("Error serializing serial vote: {}", e);
                                        return;
                                    }
                                }).await {
                                    Ok(_) => (),
                                    Err(e) => error!("Error publishing serial vote: {}", e)
                                };
                            }
                        }
                        let topic = swarm_set.topic.clone();
                        swarm.publish(topic, match NetData::SerialVote(block_info.id, block_info.hash, Vote::No).try_to_vec() {
                            Ok(v)  => v,
                            Err(e) => {
                                error!("Error serializing serial vote: {}", e);
                                return;
                            }
                        }).await.ok();
                    },
                    NetEvent::PendingBlockTimeout(hash) => {
                        {
                            let pending_blocks = swarm_set.pending_blocks.read().await;
                            for pending in pending_blocks.as_slice() {
                                if pending.0 == hash {
                                    // We need to always have majority vote
                                    if pending.1.no > pending.1.ok {
                                        approved_blocks.remove(&hash);
                                    }
                                    break;
                                }
                            }
                        }
                        swarm_set.pending_blocks.write().await.retain(|x| x.0 != hash);
                    },
                    NetEvent::ApprovedBlockTimeout(height, hash, raw_hash) => {
                        let mut quorum = false;
                        if !approved_blocks.contains_key(&raw_hash) {
                            continue;
                        }
                        {
                            let approved_blocks = swarm_set.approved_blocks.read().await;
                            for appr in approved_blocks.as_slice() {
                                if appr.1 == hash {
                                    // We need to always have majority vote
                                    if appr.2.no < appr.2.ok {
                                        quorum = true;
                                    }
                                    break;
                                }
                            }
                        }
                        if quorum {
                            swarm_set.approved_blocks.write().await.retain(|x| x.1 != raw_hash);
                            let body      = approved_blocks.remove(&raw_hash).unwrap();
                            let block     = Block { hash, body: body.0 };
                            let block_ser = match block.try_to_vec() {
                                Ok(v) => v,
                                Err(e) => {
                                    error!("Error serializing block: {}", e);
                                    continue;
                                }
                            };
                            match block::add_new(&swarm_set.db.clone(), height, block_ser.as_slice(), &hash).await {
                                Ok(_)  => (),
                                Err(e) => error!("Error occured while writing new block {}", e)
                            }
                        }
                    },
                }
            },
            None => {
                error!("Network events channel closed, exiting");
            }
        }
    };
}
/*
pub async fn new_chain(swarm: &mut Swarm<NyumtProt>, swarm_set: &Arc<RwLock<SwarmSettings>>, recv: &mut Receiver<Request>) -> Result<(), Box<dyn Error>> {
    info!("No block found, awaiting for first configuration block");
    let mut req;
    loop {
        req = get_request(recv).await?;
        if req.body.req.len() == 2 {
            match &req.body.req[0] {
                RequestType::Order(Order::AddUser { .. }) => match &req.body.req[1] {
                    RequestType::Order(Order::AddMasterNode { publickey, .. }) => {
                        let rez = swarm.local_peer_id().is_public_key(&match publickey.to_publickey() {
                            Ok(v)  => v,
                            Err(_) => {
                                req.resp_channel.send(RequestStatus::PubKeyError).ok();
                                continue;
                            }
                        });
                        if rez.is_some() && rez.unwrap() == true {
                            let (bytes, hash) = serialize_block(&req.body, None, None)?;
                            match block::add_new(&swarm_set.read().await.db, 0, bytes.as_slice(), &hash).await {
                                Ok(_)  => (),
                                Err(e) => error!("Error occured while writing new block {}", e)
                            }
                            req.resp_channel.send(RequestStatus::Ok).ok();
                            break;
                        }
                        error!("First block: Wrong public key ({}), public key must be of the node", publickey.to_base58());
                        continue;
                    }
                    _ => ()
                },
                _ => ()
            }
        }
        error!("Wrong order. Chain must be configured first");
        req.resp_channel.send(RequestStatus::NotConfigured).ok();
    };
    Ok(())
}
*/
#[derive(Debug, Clone)]
pub struct SwarmHandlerCall {
    pub tx: Sender<SwarmHandlerReq>
}

impl SwarmHandlerCall {
    pub async fn publish(&mut self, topic: Topic, data: Vec<u8>) -> Result<MessageId, PublishError> {
        match self.call(SwarmHandlerRequest::Publish(topic, data)).await {
            SwarmHandlerResponse::Publish(v) => v,
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn config_update(&mut self, peer: PeerId, data: Vec<u8>) -> Result<(), PublishError> {
        match self.call(SwarmHandlerRequest::ConfigUpdate(peer, data)).await {
            SwarmHandlerResponse::ConfigUpdate => Ok(()),
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn master_node_list_update(&mut self, peer: PeerId, data: Vec<u8>) -> Result<(), PublishError> {
        match self.call(SwarmHandlerRequest::MasterNodeListUpdate(peer, data)).await {
            SwarmHandlerResponse::MasterNodeListUpdate => Ok(()),
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn address(&mut self) -> Multiaddr {
        match self.call(SwarmHandlerRequest::Address).await {
            SwarmHandlerResponse::Address(v) => v,
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn respond(&mut self, channel: ResponseChannel<super::net_behavior::NyumtResp>, data: super::net_behavior::NyumtResp) -> () {
        match self.call(SwarmHandlerRequest::Response(channel, data)).await {
            SwarmHandlerResponse::None => (),
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn peers(&mut self) -> u32 {
        match self.call(SwarmHandlerRequest::MasterPeersCount).await {
            SwarmHandlerResponse::MasterPeersCount(v) => v,
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn peer_present(&mut self, peer: PeerId) -> bool {
        match self.call(SwarmHandlerRequest::PeerPresent(peer)).await {
            SwarmHandlerResponse::PeerPresent(v) => v,
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn get_stats(&mut self, swarm_set: &Arc<SwarmSettings>, peer: PeerId, oid: OID) -> Option<PeerResponse> {
        let req_id;
        match self.call(SwarmHandlerRequest::Request(peer, NyumtReq(PeerRequest::GetStats(oid).try_to_vec().expect("Serializing should not fail")))).await {
            SwarmHandlerResponse::Request(v) => req_id = v,
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
        let (tx, rx) = oneshot::channel();
        swarm_set.resp.write().await.insert(req_id.clone(), tx);
    
        match timeout(swarm_set.timeout.clone(), rx).await {
            Ok(v) => {
                return Some(v.unwrap())
            },
            Err(_) => ()
        }

        None
    }

    pub async fn disconnect_from_peer(&mut self, peer: &PeerId) -> Result<(), ()> {
        match self.call(SwarmHandlerRequest::DisconnectPeer(peer.clone())).await {
            SwarmHandlerResponse::DisconnectPeerStatus(v) => v,
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn subscribe(&mut self, topic: Topic) {
        match self.call(SwarmHandlerRequest::Subscribe(topic)).await {
            SwarmHandlerResponse::Subscribe => (),
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    async fn call(&mut self, req: SwarmHandlerRequest) -> SwarmHandlerResponse {
        let (tx, rx) = oneshot::channel();
        match self.tx.send(SwarmHandlerReq { req, tx }).await {
            Ok(_) => {
                match rx.await {
                    Ok(v) => v,
                    Err(e) => {
                        panic!("Fatal error with swarm channel: {}", e);
                    }
                } 
            }
            Err(e) => {
                panic!("Fatal error with swarm channel: {}", e);
            }
        }
    }
}

enum SwarmHandlerResponse {
    Publish(Result<MessageId, PublishError>),
    MasterPeersCount(u32),
    DisconnectPeerStatus(Result<(), ()>),
    ConfigUpdate,
    MasterNodeListUpdate,
    Subscribe,
    PeerPresent(bool),
    Address(Multiaddr),
    Request(RequestId),
    None
}

pub struct SwarmHandlerReq {
    req: SwarmHandlerRequest,
    tx: oneshot::Sender<SwarmHandlerResponse>
}

enum SwarmHandlerRequest {
    Publish(Topic, Vec<u8>),
    MasterPeersCount,
    DisconnectPeer(PeerId),
    Subscribe(Topic),
    ConfigUpdate(PeerId, Vec<u8>),
    MasterNodeListUpdate(PeerId, Vec<u8>),
    PeerPresent(PeerId),
    Address,
    Request(PeerId, NyumtReq),
    Response(ResponseChannel<super::net_behavior::NyumtResp>, super::net_behavior::NyumtResp)
}

pub async fn sync(swarm_set: &Arc<SwarmSettings>, recv: &mut Receiver<Request>,
                  rx: Receiver<NetEvent>, tx: Sender<NetEvent>,
                  swarm_handler: SwarmHandlerCall, local_pk: PublicKey) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut req;
    let topic = swarm_set.topic.clone();

    {
        let swrm_set_clone = swarm_set.clone();
        let swrm_handle    = swarm_handler.clone();
        tokio::task::spawn_blocking(move || {
            futures::executor::block_on(handle_incoming_data(swrm_handle, swrm_set_clone, rx, tx));
        });
    }

    let global_db = swarm_set.db.clone();
    loop {
        req = get_request(recv).await?;

        let swrm_set         = swarm_set.clone();
        let topic_clone      = topic.clone();
        let mut swrm_handler = swarm_handler.clone();
        let db               = global_db.clone();
        let pk               = local_pk.clone();
        

        tokio::spawn(async move {
            match req.body {
                ChannelRequestType::GetStats(peer_id, oid) => {
                    get_stats(swrm_set, peer_id, oid, req.resp_channel).await;
                },
                ChannelRequestType::Block(body) => {
                    let (mut height, last_hash) = match block::get_last_block(&db).await {
                        Ok(Some(v)) => v,
                        _           => {
                            req.resp_channel.send(RequestStatus::NodeError).ok();
                            error!("Error reading from database. database may have been externally modified while node running.");
                            return;
                        }
                    };
                    height += 1;
                    let (bytes, hash1) = match serialize_req(&body) {
                        Ok(v)  => v,
                        Err(e) => {
                            req.resp_channel.send(RequestStatus::NodeError).ok();
                            error!("Error serializing block: {}", e);
                            return;
                        }
                    };
                    match publish(&mut swrm_handler, &swrm_set, &topic_clone, bytes.as_slice(), height, &hash1).await {
                        Ok(v) => {
                            let mut hash: Option<[u8; 32]> = None;
                            let mut hasher = None;
                            match v {
                                Some(v) => {
                                    hash   = Some(v.1);
                                    height = v.0;
                                },
                                None    => {
                                    let mut _hasher = Sha256::new();
                                    _hasher.update(&last_hash);
                                    hasher = Some(_hasher);
                                }
                            }

                            let (block, hash) = match serialize_block(&body, hash, hasher) {
                                Ok(v)  => v,
                                Err(e) => {
                                    req.resp_channel.send(RequestStatus::NodeError).ok();
                                    error!("Error serializing block: {}", e);
                                    return;
                                }
                            };
                            req.resp_channel.send(RequestStatus::Ok).ok();
                            info!("New block with height {} and hash {:x?}", height, hash);
                            match block::add_new(&db, height, block.as_slice(), &hash).await {
                                Ok(_)  => (),
                                Err(e) => {
                                    error!("Error occured while writing new block {}", e);
                                    return;
                                }
                            }

                            let updates  = match db_interpret::interpret_block(&pk, &db.con, &body.req).await {
                                Ok(v) => v,
                                Err(e) => {
                                    error!("Error saving updates to database: {}", e);
                                    return;
                                }
                            };

                            if !swrm_set.subscribed_nodes.read().await.is_empty() {
                                let swrm_set_clone = swrm_set.clone();
                                tokio::task::spawn_blocking(move || {
                                    futures::executor::block_on(send_update(updates, swrm_set_clone));
                                });
                            }
                        },
                        Err(BlockPublishError::DupReq)   => { req.resp_channel.send(RequestStatus::DupReq).ok(); },
                        Err(BlockPublishError::Rejected) => { req.resp_channel.send(RequestStatus::Rejected).ok(); },
                        Err(_)                           => { req.resp_channel.send(RequestStatus::ReqError).ok(); }
                    }
                },
            }
        });
    };
}

pub async fn get_stats(swarm_set: Arc<SwarmSettings>, peer_id: PeerId, oid: OID, resp_channel: oneshot::Sender<RequestStatus>) {
    let mut handle = swarm_set.swarm.clone();
    let my_peer_id = swarm_set.pubkey.to_peer_id();

    if my_peer_id.is_ok() && my_peer_id.unwrap().eq(&peer_id) {
        let sys       = Arc::new(System::new());
        let db_struct = Arc::new(DbStruct::from_oid(&oid));

        match generate_report(&sys, &db_struct).await {
            Ok(rep) => {
                match rep.try_to_vec() {
                    Ok(v) => {
                        match swarm_set.keypair.sign(&v) {
                            Ok(v) => {
                                resp_channel.send(RequestStatus::GetStats(rep, v)).ok();
                                return;
                            },
                            Err(_) => ()
                        }
                    },
                    Err(_) => ()
                }
            },
            Err(_) => ()
        }
        resp_channel.send(RequestStatus::ReqError).ok();
        return;
    } else if handle.peer_present(peer_id.clone()).await {
        let resp = handle.get_stats(&swarm_set, peer_id, oid).await;
        match resp {
            Some(v) => {
                match v {
                    PeerResponse::GetStats(rep, sig) => {
                        resp_channel.send(RequestStatus::GetStats(rep, sig)).ok();
                        return;
                    },
                    _ => {
                        resp_channel.send(RequestStatus::ReqError).ok();
                        return;
                    }
                }
            },
            None => {
                resp_channel.send(RequestStatus::RequestTimeout).ok();
                return;
            }
        }
    }
    resp_channel.send(RequestStatus::NodeInactive).ok();
}

pub async fn send_update(updates: Vec<Update>, swrm_set: Arc<SwarmSettings>) {
    let db = swrm_set.db.clone();
    let pk = swrm_set.pubkey.clone();

    for update in updates {
        match update {
            Update::ConfigUpdate(name, master, node) => {
                if swrm_set.conf_name.read().await.eq(&name) {
                    let mut lock = swrm_set.conf.write().await;
                    lock.master = master.clone();
                    lock.node   = node.clone();
                    lock.update();
                }
                match swrm_set.subscribed_nodes.read().await.get(&name) {
                    Some(v) => {
                        let update = match PeerRequest::UpdateConfig(node).try_to_vec() {
                            Ok(v) => v,
                            Err(e) => {
                                error!("Error serializing config update to be sent to nodes: {}", e);
                                continue;
                            }
                        };
                        for i in v.as_slice() {
                            swrm_set.swarm.clone().config_update(i.clone(), update.clone()).await.ok();
                        }
                    },
                    None => ()
                }
            },
            Update::NodeConfigChange(pubkey, name) => {
                let conf = match cache::get_node_conf(&db, &name).await {
                    Ok(Some(v)) => {
                        let master_conf = match MasterConfig::try_from_slice(&v.1) {
                            Ok(v) => v,
                            Err(_) => continue
                        };
                        let node_conf = match NodeConfig::try_from_slice(&v.2) {
                            Ok(v) => v,
                            Err(_) => continue
                        };
                        (v.0, master_conf, node_conf)
                    },
                    _ => continue
                };
                if pk == pubkey {
                    let mut lock = swrm_set.conf.write().await;
                    lock.master = conf.1;
                    lock.node   = conf.2;
                    lock.update();
                    *swrm_set.conf_name.write().await = name;
                } else {
                    let peer_id = pubkey.to_peer_id().unwrap();
                    for i in swrm_set.subscribed_nodes.read().await.values() {
                        if i.contains(&peer_id) {
                            let update = match PeerRequest::UpdateConfig(conf.2.clone()).try_to_vec() {
                                Ok(v) => v,
                                Err(e) => {
                                    error!("Error serializing config update to be sent to nodes: {}", e);
                                    continue;
                                }
                            };
                            swrm_set.swarm.clone().config_update(peer_id, update).await.ok();
                            break;
                        }
                    }
                }
            },
            Update::MasterNodeListUpdate => {
                let db       = swrm_set.db.clone();
                let mut list = vec![];
                let mut iter = node::get_master_nodes(&db).await;
                while let Some(mnode) = iter.next().await {
                    match mnode {
                        Ok(v) => {
                            let mpk = match PublicKey::try_from_slice(&v.public_key) {
                                Ok(_pk) => _pk,
                                Err(e) => {
                                    error!("Error serializing master node public key: {}", e);
                                    continue;
                                }
                            };
                            list.push(MasterNode {
                                addr: if mpk == swrm_set.pubkey { Some(swrm_set.swarm.clone().address().await.to_string()) } else { None },
                                pubkey: mpk
                            });
                        },
                        Err(e) => {
                            error!("Error fetching master node list from database: {}", e);
                            continue;
                        }
                    }
                }
                let update = match PeerRequest::UpdateMasterNodeList(MasterNodes(list)).try_to_vec() {
                    Ok(v) => v,
                    Err(e) => {
                        error!("Error serializing master node list update to be sent to nodes: {}", e);
                        continue;
                    }
                };
                for i in swrm_set.subscribed_nodes.read().await.values() {
                    for node in i {
                        swrm_set.swarm.clone().master_node_list_update(node.clone(), update.clone()).await.ok();
                    }
                }
            },
            Update::None => ()
        }
    }
}

pub async fn swarm_event_handler(mut swarm: Swarm<NyumtProt>, mut rx: Receiver<SwarmHandlerReq>) {
    loop {
        tokio::select! {
            rec = rx.recv() => {
                match rec {
                    Some(event) => {
                        match event.req {
                            SwarmHandlerRequest::Request(peer_id, req) => {
                                event.tx.send(SwarmHandlerResponse::Request(swarm.behaviour_mut().req_resp.send_request(&peer_id, req))).ok();
                            },
                            SwarmHandlerRequest::Response(ch, resp) => {
                                swarm.behaviour_mut().req_resp.send_response(ch, resp).ok();
                                event.tx.send(SwarmHandlerResponse::None).ok();
                            },
                            SwarmHandlerRequest::PeerPresent(peer) => {
                                event.tx.send(SwarmHandlerResponse::PeerPresent(swarm.is_connected(&peer))).ok();
                            },
                            SwarmHandlerRequest::MasterPeersCount => {
                                let mut count = 0;
                                for _ in swarm.behaviour().gossipsub.all_mesh_peers() {
                                    count += 1;
                                }
                                event.tx.send(SwarmHandlerResponse::MasterPeersCount(count)).ok();
                            },
                            SwarmHandlerRequest::ConfigUpdate(peer_id, update) => {
                                swarm.behaviour_mut().req_resp.send_request(&peer_id, NyumtReq(update));
                                event.tx.send(SwarmHandlerResponse::ConfigUpdate).ok();
                            },
                            SwarmHandlerRequest::MasterNodeListUpdate(peer_id, update) => {
                                swarm.behaviour_mut().req_resp.send_request(&peer_id, NyumtReq(update));
                                event.tx.send(SwarmHandlerResponse::MasterNodeListUpdate).ok();
                            },
                            SwarmHandlerRequest::Publish(topic, data) => {
                                event.tx.send(SwarmHandlerResponse::Publish(swarm.behaviour_mut().gossipsub.publish(topic, data))).ok();
                            },
                            SwarmHandlerRequest::DisconnectPeer(peer) => {
                                event.tx.send(SwarmHandlerResponse::DisconnectPeerStatus(swarm.disconnect_peer_id(peer))).ok();
                            },
                            SwarmHandlerRequest::Subscribe(topic) => {
                                swarm.behaviour_mut().gossipsub.subscribe(&topic).ok();
                                event.tx.send(SwarmHandlerResponse::Subscribe).ok();
                            },
                            SwarmHandlerRequest::Address => {
                                let mut addrs = swarm.external_addresses();
                                while let Some(addr) = addrs.next() {
                                    event.tx.send(SwarmHandlerResponse::Address(addr.addr.clone())).ok();
                                    break;
                                }
                            }
                        }
                    }
                    None => error!("Swarm event handler suddenly closed, but it shouldn't")
                }
            },
            event = swarm.select_next_some() => {
                match event {
                    libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {
                        info!("Master node listening on {:?}", address);
                    },
                    libp2p::swarm::SwarmEvent::IncomingConnection { local_addr, .. } => {
                        let addr = swarm.behaviour().sett.addr.clone();
                        if futures::executor::block_on(addr.read()).is_none() {
                            *futures::executor::block_on(addr.write()) = Some(local_addr);
                        }
                    },
                    libp2p::swarm::SwarmEvent::ConnectionClosed { peer_id, .. } => {
                        // Remove subscriber
                    },
                    _ => ()
                }
            }
        }
    };
}
