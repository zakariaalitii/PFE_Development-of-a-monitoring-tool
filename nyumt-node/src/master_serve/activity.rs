use std::{
    error::Error,
    sync::Arc, collections::HashMap
};
use libp2p::{Swarm, PeerId, request_response::ResponseChannel};
use tokio::sync::{
    mpsc::{Receiver, Sender},
    oneshot,
    RwLock
};
use borsh::{BorshSerialize, BorshDeserialize};
use tracing::{error, info};
use futures::StreamExt;
use nyumt_proto::{api::{ClientResponse, SessionSettings, RequestBlock, Stats, Stat, ClientRequest}, types::Permissions};

use super::behavior::{NyumtReq, NyumtServerProt};
use crate::{
    CLIENT_POLL_TIME,
    database::{Database, auth, block, node},
    utils::keys::PublicKey
};


#[derive(Debug, Clone)]
pub struct SwarmHandlerCall {
    pub tx: Sender<SwarmHandlerReq>
}

impl SwarmHandlerCall {
    pub async fn send_reports(&mut self, peer_id: PeerId, reports: Vec<u8>) -> Result<(), ()> {
        match self.call(SwarmHandlerRequest::Report(peer_id, reports)).await {
            SwarmHandlerResponse::Report => Ok(()),
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn send_stats(&mut self, peer_id: PeerId, stats: Vec<u8>) -> Result<(), ()> {
        match self.call(SwarmHandlerRequest::Stat(peer_id, stats)).await {
            SwarmHandlerResponse::Stat => Ok(()),
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn req_error(&mut self, peer: &PeerId) -> Result<(), ()> {
        match self.call(SwarmHandlerRequest::Error(peer.clone())).await {
            SwarmHandlerResponse::Error => Ok(()),
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn disconnect_from_peer(&mut self, peer: &PeerId) -> Result<(), ()> {
        match self.call(SwarmHandlerRequest::DisconnectPeer(peer.clone())).await {
            SwarmHandlerResponse::DisconnectPeerStatus(v) => v,
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn respond(&mut self, channel: ResponseChannel<super::behavior::NyumtResp>, data: super::behavior::NyumtResp) -> () {
        match self.call(SwarmHandlerRequest::Response(channel, data)).await {
            SwarmHandlerResponse::None => (),
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
    Report,
    Stat,
    DisconnectPeerStatus(Result<(), ()>),
    Error,
    None
}

pub struct SwarmHandlerReq {
    req: SwarmHandlerRequest,
    tx: oneshot::Sender<SwarmHandlerResponse>
}

enum SwarmHandlerRequest {
    Report(PeerId, Vec<u8>),
    Stat(PeerId, Vec<u8>),
    DisconnectPeer(PeerId),
    Error(PeerId),
    Response(ResponseChannel<super::behavior::NyumtResp>, super::behavior::NyumtResp)
}

pub async fn swarm_event_handler(mut swarm: Swarm<NyumtServerProt>, mut rx: Receiver<SwarmHandlerReq>) {
    loop {
        tokio::select! {
            rec = rx.recv() => {
                match rec {
                    Some(event) => {
                        match event.req {
                            SwarmHandlerRequest::Response(ch, resp) => {
                                swarm.behaviour_mut().req_resp.send_response(ch, resp).ok();
                                event.tx.send(SwarmHandlerResponse::None).ok();
                            },
                            SwarmHandlerRequest::Report(peer_id, report) => {
                                swarm.behaviour_mut().req_resp.send_request(&peer_id, NyumtReq(report));
                                event.tx.send(SwarmHandlerResponse::Report).ok();
                            },
                            SwarmHandlerRequest::Stat(peer_id, stat) => {
                                swarm.behaviour_mut().req_resp.send_request(&peer_id, NyumtReq(stat));
                                event.tx.send(SwarmHandlerResponse::Stat).ok();
                            },
                            SwarmHandlerRequest::DisconnectPeer(peer_id) => {
                                event.tx.send(SwarmHandlerResponse::DisconnectPeerStatus(swarm.disconnect_peer_id(peer_id))).ok();
                            }
                            SwarmHandlerRequest::Error(peer_id) => {
                                match ClientResponse::Error.try_to_vec() {
                                    Ok(v) => {
                                        swarm.behaviour_mut().req_resp.send_request(&peer_id, NyumtReq(v));
                                    },
                                    Err(e) => {
                                        error!("Error serializing simple error, closing connection: {}", e);
                                        swarm.disconnect_peer_id(peer_id).ok();
                                    }
                                };
                                event.tx.send(SwarmHandlerResponse::Error).ok();
                            }
                        }
                    },
                    None => error!("Swarm event handler suddenly closed")
                }
            }
            event = swarm.select_next_some() => {
                match event {
                    libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {
                        info!("Client listener on {:?}", address);
                    },
                    libp2p::swarm::SwarmEvent::ConnectionClosed { peer_id, endpoint, .. } => {
                        match swarm.behaviour_mut().sett.auth.write().await.remove(&peer_id) {
                            Some(v) => {
                                info!("User {} (addr {} and public key {}) disconnected", v.name, endpoint.get_remote_address(), v.pubkey);
                                v.disconnected.send(()).ok();
                            },
                            None    => ()
                        }
                    }
                    _ => ()
                }
            }
        }
    };
}

pub async fn handler(swarm: Swarm<NyumtServerProt>, swarm_rx: Receiver<SwarmHandlerReq>) -> Result<(), Box<dyn Error + Send + Sync>> {
    swarm_event_handler(swarm, swarm_rx).await;

    Ok(())
}

pub async fn event_handler(mut swarm_handle: SwarmHandlerCall, db: Database,
                           auth: Arc<RwLock<HashMap<PeerId, super::types::User>>>, peer_id: PeerId, pk: PublicKey,
                           mut client_settings: SessionSettings) {
    let user;
    let (mut is_disconnected, disconnect);

    {
        let db_user = match auth::get_user_with_pubkey(&db, &match pk.try_to_vec() {
            Ok(v) => v,
            Err(_) => {
                error!("Error serializing public key, closing connection with {}", pk);
                swarm_handle.disconnect_from_peer(&peer_id).await.ok();
                return;
            }
        }).await {
            Ok(v) => v,
            Err(_) => {
                error!("Error connecting to database, closing connection with {}", pk);
                swarm_handle.disconnect_from_peer(&peer_id).await.ok();
                return;
            }
        };

        (disconnect, is_disconnected) = oneshot::channel();

        user = super::types::User {
            name: db_user.name,
            permissions: match Permissions::try_from_slice(&db_user.permissions) {
                Ok(v) => v,
                Err(_) => {
                    error!("Error serializing permissions, closing connection with {}", pk);
                    swarm_handle.disconnect_from_peer(&peer_id).await.ok();
                    return;
                }
            },
            pubkey: pk,
            last_access: db_user.last_access as u64,
            disconnected: disconnect

        };
    }

    auth.write().await.insert(peer_id.clone(), user);

    loop {
        tokio::select! {
            _ = &mut is_disconnected => break,
            height = block::get_last_block_height(&db) => {
                match height {
                    Ok(Some(last)) => {
                        if last as i64 > client_settings.last_request_height {
                            let mut block_reader = block::get_blocks_from(&db, client_settings.last_request_height).await;
                            let mut blocks       = Vec::with_capacity(10);
                            loop {
                                let next = block_reader.next().await;
                                match next {
                                    Some(v) => match v {
                                        Ok(v) => {
                                            blocks.push(RequestBlock { id: v.0, body: v.1 });
                                            if last == blocks.last().unwrap().id || blocks.len() >= 9 {
                                                match ClientRequest::Block(blocks).try_to_vec() {
                                                    Ok(v) => {
                                                        swarm_handle.send_reports(peer_id.clone(), v).await.ok();
                                                    },
                                                    Err(e) => {
                                                        error!("Error serializing blocks to be sent to user: {}", e);
                                                        break;
                                                    }
                                                }
                                                blocks = vec![];
                                            }
                                            client_settings.last_request_height += 1;
                                        },
                                        Err(e) => {
                                            error!("Error occured while reading blocks from database: {}", e);
                                            break;
                                        }
                                    },
                                    None => break
                                }
                            }
                        }
                    }
                    _ => ()
                }
            },
            stats = node::poll_stats_ready(&db, client_settings.last_timestamp) => {
                match stats {
                    Ok(ready) => {
                        if ready {
                            let mut stat_reader = node::get_stats_from(&db, client_settings.last_timestamp).await;
                            let mut stats       = Stats(Vec::with_capacity(10));
                            let mut last        = 0;
                            loop {
                                let next = stat_reader.next().await;
                                match next {
                                    Some(v) => match v {
                                        Ok(tup) => {
                                            if stats.0.len() >= 9 {
                                                match ClientRequest::Stats(stats).try_to_vec() {
                                                    Ok(v) => {
                                                        swarm_handle.send_stats(peer_id.clone(), v).await.ok();
                                                        client_settings.last_timestamp = last;
                                                        stats = Stats(Vec::with_capacity(30));
                                                    },
                                                    Err(e) => {
                                                        error!("Error serializing node stats: {}", e);
                                                        break;
                                                    }
                                                }
                                            }
                                            stats.0.push(Stat { id: tup.0, data: tup.1, sig: tup.2 });
                                            last = tup.3;
                                        },
                                        Err(e) => {
                                            error!("Error occured while reading from database: {}", e);
                                            break;
                                        },
                                    },
                                    None => {
                                        if stats.0.len() > 0 {
                                            match ClientRequest::Stats(stats).try_to_vec() {
                                                Ok(v) => {
                                                    swarm_handle.send_stats(peer_id.clone(), v).await.ok();
                                                    client_settings.last_timestamp = last;
                                                },
                                                Err(e) => error!("Error serializing node stats: {}", e)
                                            }
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                    },
                    _ => ()
                }
            }
        }
        tokio::time::sleep(*CLIENT_POLL_TIME).await;
    };
}
