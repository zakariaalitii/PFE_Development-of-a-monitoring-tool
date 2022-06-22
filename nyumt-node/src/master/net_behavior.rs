use std::{io, sync::Arc};
use futures::{prelude::*, AsyncWriteExt};
use libp2p::{
    swarm::NetworkBehaviourEventProcess,
    mdns::{Mdns, MdnsEvent},
    NetworkBehaviour,
    request_response::{ RequestResponse, RequestResponseCodec, ProtocolName, RequestResponseEvent, RequestResponseMessage },
    gossipsub::{ self, GossipsubEvent }, core::upgrade::{write_length_prefixed, read_length_prefixed},
    ping
};
use borsh::{BorshSerialize, BorshDeserialize};
use tracing::{warn, debug, error};
use async_trait::async_trait;
use nyumt_proto::block::{ verify_hash, verify_req };

use crate::{master::{
    SwarmSettings, types::{ NetEvent, Block, Vote, ArchiveBlock, BodyData, Permissions },
    TotalVote
}, database::node};
use crate::database::auth;
//use crate::node::req_resp;
use crate::node::req_resp;
use crate::utils::keys::PublicKey;

#[repr(u8)]
#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub enum NetData {
    /// The block where orders, info, ... exist
    Block(Block),
    /// Voting whether to accept block or no
    Vote([u8; 32], Vote),
    /// Demand for block unique id and hash
    ArchiveBlock(ArchiveBlock),
    /// Deciding block id and hash
    SerialVote(u64, [u8; 32], Vote)
}

#[derive(NetworkBehaviour)]
#[behaviour(event_process = true)]
pub struct NyumtProt {
    pub gossipsub: gossipsub::Gossipsub,
    pub mdns: Mdns,
    pub req_resp: RequestResponse<NyumtReqRespCodec>,
    pub ping: ping::Behaviour,

    #[behaviour(ignore)]
    #[allow(dead_code)]
    pub sett: Arc<SwarmSettings>
}

impl NetworkBehaviourEventProcess<ping::PingEvent> for NyumtProt {
    fn inject_event(&mut self, event: ping::PingEvent) {
    }
}

impl NetworkBehaviourEventProcess<MdnsEvent> for NyumtProt {
    fn inject_event(&mut self, event: MdnsEvent) {
        if let MdnsEvent::Discovered(list) = event {
            for (peer_id, multiaddr) in list {
                //self.gossipsub.add_explicit_peer(&peer_id);
            }
        }
    }
}

#[async_trait]
impl NetworkBehaviourEventProcess<RequestResponseEvent<NyumtReq, NyumtResp>> for NyumtProt {
    fn inject_event(&mut self, message: RequestResponseEvent<NyumtReq, NyumtResp>) {
        match message {
            RequestResponseEvent::Message { peer, message: RequestResponseMessage::Request { request, channel, ..} } => {
                let sett = self.sett.clone();
                tokio::spawn(async move {
                    req_resp::handle(&request.0, peer, req_resp::NodeLevel::Master(sett, channel)).await;
                });
            },
            RequestResponseEvent::Message { peer, message: RequestResponseMessage::Response { response, request_id, ..} } => {
                let sett = self.sett.clone();
                tokio::spawn(async move {
                    req_resp::handle_response(&response.0, peer, request_id, req_resp::NodeLevelResponse::Master(sett)).await;
                });
            },
            _ => ()
        }
    }
}

impl NetworkBehaviourEventProcess<GossipsubEvent> for NyumtProt {
    // Called when `floodsub` produces an event.
    fn inject_event(&mut self, message: GossipsubEvent) {
        match message {
            GossipsubEvent::Message { message, message_id, .. } => {
                match message.source {
                    Some(peer) => {
                        let mut deser       = None;
                        let mut knewn_peer_ = None;
                        {
                            let db = self.sett.db.clone();
                            match futures::executor::block_on(node::get_peer_master_node_pk(&db, &peer.to_string())) {
                                Ok(Some(v)) => {
                                    let pk = match PublicKey::try_from_slice(&v) {
                                        Ok(v) => v,
                                        Err(e) => {
                                            error!("Error parsing peer public key: {}", e);
                                            return;
                                        }
                                    };
                                    let knewn_peer = match pk.to_publickey() {
                                        Ok(v) => v,
                                        Err(e) => {
                                            error!("Error parsing peer public key: {}", e);
                                            return;
                                        }
                                    };
                                    if knewn_peer.verify(message.data.as_slice(), message_id.0.as_slice()) {
                                        deser      = Some(match NetData::try_from_slice(message.data.as_slice()) {
                                            Ok(v)  => {
                                                knewn_peer_ = Some(knewn_peer);
                                                v
                                            },
                                            Err(e) => {
                                                error!("Error deserializing incoming network data from peer {}: {}", pk, e);
                                                return;
                                            }
                                        });
                                    }
                                },
                                _ => ()
                            }
                        }

                        match deser {
                            Some(data) => {
                                //let knewn_peer = knewn_peer_.unwrap();
                                match data {
                                    NetData::Vote(hash, vote) => {
                                        let channel = self.sett.channel.clone();
                                        futures::executor::block_on(channel.send(NetEvent::Vote(hash, vote, 1))).ok();
                                    },
                                    NetData::SerialVote(height, hash, vote) => {
                                        let channel = self.sett.channel.clone();
                                        futures::executor::block_on(channel.send(NetEvent::SerialVote(height, hash, vote, 1))).ok();
                                    },
                                    NetData::ArchiveBlock(v) => {
                                        // Request to vote for block height and sequenced hash
                                        let channel = self.sett.channel.clone();
                                        futures::executor::block_on(channel.send(NetEvent::ArchiveBlock(v))).ok();
                                    },
                                    NetData::Block(block) => {
                                        let pending_blocks = self.sett.pending_blocks.clone();
                                        futures::executor::block_on(pending_blocks.write()).push((block.hash.to_owned(), TotalVote { ok: 0, no: 0 }, 0));
                                        // Validate block first before voting
                                        if futures::executor::block_on(verify_hash(&block.body, &block.hash)) {
                                            // checking if requests in block are valid
                                            match BodyData::try_from_slice(&block.body) {
                                                Ok(v) => {
                                                    let db = self.sett.db.clone();
                                                    let user_details;
                                                    {
                                                        match v.user_pk.try_to_vec() {
                                                            Ok(v) => user_details = Some(futures::executor::block_on(auth::get_user_with_pubkey(&db, &v))),
                                                            Err(_) => {
                                                                user_details = None;
                                                                error!("Error parsing user's public key ({})", v.user_pk)
                                                            }
                                                        }
                                                    }
                                                    if user_details.is_some() {
                                                        match user_details.unwrap() {
                                                            Ok(user) => {
                                                                match Permissions::try_from_slice(&user.permissions) {
                                                                    Ok(perm) => {
                                                                        match futures::executor::block_on(verify_req(&db.con, &block.body, &v.req, &v.user_pk, &perm, &v.sign)) {
                                                                            Ok(None) => {
                                                                                // Valid block
                                                                                // Appending it to waitlist
                                                                                {
                                                                                    let mut pending_blocks = futures::executor::block_on(self.sett.pending_blocks.write());
                                                                                    for pending in pending_blocks.as_mut_slice() {
                                                                                        if pending.0 == block.hash {
                                                                                            pending.1.ok  += 1;
                                                                                            pending.2     += 1;
                                                                                        }
                                                                                    }
                                                                                }

                                                                                let topic;
                                                                                let retry;
                                                                                {
                                                                                    topic = self.sett.topic.clone();
                                                                                    retry = futures::executor::block_on(self.sett.conf.read()).master.vote_push_retries;
                                                                                }
                                                                                match NetData::Vote(block.hash.clone(), Vote::Ok).try_to_vec() {
                                                                                    Ok(vote)  => {
                                                                                        for i in 1..=retry {
                                                                                            match self.gossipsub.publish(topic.clone(), vote.as_slice()) {
                                                                                                Ok(_)  => {
                                                                                                        let channel = self.sett.channel.clone();
                                                                                                        futures::executor::block_on(channel.send(NetEvent::NewBlock { body: v, hash: block.hash  })).ok();
                                                                                                        return;
                                                                                                }
                                                                                                Err(e) => {
                                                                                                    if i == retry {
                                                                                                        error!("Error voting for block validity: {}", e);
                                                                                                    }
                                                                                                } 
                                                                                            }
                                                                                        }
                                                                                    },
                                                                                    Err(e) => error!("Error serializing vote netdata: {}", e)
                                                                                }
                                                                            },
                                                                            _ => ()
                                                                        }
                                                                    }
                                                                    Err(e) => error!("Error parsing user with public key {} permissions: {}", v.user_pk, e)
                                                                }
                                                            },
                                                            Err(_) => ()
                                                        }
                                                    }
                                                },
                                                Err(_) => debug!("Error parsing block from peer {}", peer.to_string())
                                            }
                                        }
                                        match NetData::Vote(block.hash.clone(), Vote::No).try_to_vec() {
                                            Ok(v) => {
                                                self.gossipsub.publish(self.sett.topic.clone(), v);
                                            },
                                            Err(e) => error!("Error serializing vote: {}", e)
                                        };
                                        futures::executor::block_on(self.sett.pending_blocks.write()).retain(|x| x.0 != block.hash);
                                    },
                                }
                            },
                            None => ()
                        }
                    },
                    None => ()
                }
                
                println!(
                    "Received: '{:?}' from {:?}",
                    String::from_utf8_lossy(&message.data),
                    message.source.unwrap()
                );
            }
            GossipsubEvent::Subscribed { peer_id, .. } => {
                let db = self.sett.db.clone();
                match futures::executor::block_on(node::is_peer_master_node(&db, &peer_id.to_string())) {
                    Ok(true) => debug!("Peer is connected"),
                    _ => {
                        warn!("Peer with unknown publickey trying to connect");
                        self.gossipsub.blacklist_peer(&peer_id);
                    }
                }
            }
            _ => ()
        }
    }
}

#[derive(Debug, Clone)]
pub struct NyumtReqRespProt();
#[derive(Clone)]
pub struct NyumtReqRespCodec();
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NyumtReq(pub Vec<u8>);
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NyumtResp(pub Vec<u8>);

impl ProtocolName for NyumtReqRespProt {
    fn protocol_name(&self) -> &[u8] {
        "/nyumt".as_bytes()
    }
}

#[async_trait]
impl RequestResponseCodec for NyumtReqRespCodec {
    type Protocol = NyumtReqRespProt;
    type Request = NyumtReq;
    type Response = NyumtResp;

    async fn read_request<T>(&mut self, _: &NyumtReqRespProt, io: &mut T)
        -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send
    {
        let vec = read_length_prefixed(io, 4096).await?;

        if vec.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into())
        }

        Ok(NyumtReq(vec))
    }

    async fn read_response<T>(&mut self, _: &NyumtReqRespProt, io: &mut T)
        -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send
    {
        let vec = read_length_prefixed(io, 4096).await?;

        if vec.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into())
        }

        Ok(NyumtResp(vec))
    }

    async fn write_request<T>(&mut self, _: &NyumtReqRespProt, io: &mut T, NyumtReq(data): NyumtReq)
        -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send
    {
        write_length_prefixed(io, data).await?;
        io.close().await?;

        Ok(())
    }

    async fn write_response<T>(&mut self, _: &NyumtReqRespProt, io: &mut T, NyumtResp(data): NyumtResp)
        -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send
    {
        write_length_prefixed(io, data).await?;
        io.close().await?;

        Ok(())
    }
}
