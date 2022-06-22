use std::{
    sync::Arc,
    error::Error, convert::Infallible, hash::Hasher
};
use borsh::BorshSerialize;
use libp2p::{Swarm, PeerId, request_response::{RequestId, ResponseChannel}};
use tokio::sync::{
    mpsc::{self, Receiver, Sender},
    oneshot
};
use tracing::{error, info};
use futures::StreamExt;

use crate::node::{
    net_behavior::NyumtNodeProt,
    SwarmSettings
};
use rand::{SeedableRng, prelude::SliceRandom};
use nyumt_proto::{
    peer::{PeerRequest, PeerResponse}, PROTOCOL_VERSION};
use super::net_behavior::NyumtReq;
use crate::database::{node, cache};
use crate::POLL_TIME;

#[derive(Debug, Clone)]
pub struct SwarmHandlerCall {
    pub tx: Sender<SwarmHandlerReq>
}

impl SwarmHandlerCall {
    pub async fn publish_report(&mut self, pk: Vec<u8>, timestamp: u64, report: Vec<u8>) -> Result<(), Infallible> {
        match self.call(SwarmHandlerRequest::Report(pk, timestamp, report)).await {
            SwarmHandlerResponse::Report(_) => Ok(()),
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn peers(&mut self) -> usize {
        match self.call(SwarmHandlerRequest::PeersCount).await {
            SwarmHandlerResponse::PeersCount(v) => v,
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn peers_list(&mut self) -> Vec<PeerId> {
        match self.call(SwarmHandlerRequest::Peers).await {
            SwarmHandlerResponse::Peers(v) => v,
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn synchronise(&mut self) -> Result<(), ()> {
        match self.call(SwarmHandlerRequest::Synchronise).await {
            SwarmHandlerResponse::Synchronise => Ok(()),
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn get_config(&mut self, peer: PeerId) -> RequestId {
        match self.call(SwarmHandlerRequest::Request(peer, NyumtReq(PeerRequest::NodeConfig.try_to_vec().unwrap()))).await {
            SwarmHandlerResponse::Resp(v) => v,
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn get_master_node_list(&mut self, peer: PeerId) -> RequestId {
        match self.call(SwarmHandlerRequest::Request(peer, NyumtReq(PeerRequest::MasterNodeList.try_to_vec().unwrap()))).await {
            SwarmHandlerResponse::Resp(v) => v,
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn subscribe(&mut self, peer: PeerId) -> RequestId {
        match self.call(SwarmHandlerRequest::Request(peer, NyumtReq(PeerRequest::HelloSubscribe.try_to_vec().unwrap()))).await {
            SwarmHandlerResponse::Resp(v) => v,
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
    Report(()),
    PeersCount(usize),
    Peers(Vec<PeerId>),
    Synchronise,
    Resp(RequestId),
    None
}

pub struct SwarmHandlerReq {
    req: SwarmHandlerRequest,
    tx: oneshot::Sender<SwarmHandlerResponse>
}

enum SwarmHandlerRequest {
    Report(Vec<u8>, u64, Vec<u8>),
    Peers,
    PeersCount,
    Synchronise,
    Request(PeerId, NyumtReq),
    Response(ResponseChannel<super::net_behavior::NyumtResp>, super::net_behavior::NyumtResp)
}

async fn swarm_event_handler(mut swarm: Swarm<NyumtNodeProt>, mut rx: Receiver<SwarmHandlerReq>) {
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
                            SwarmHandlerRequest::Request(peer, req) => {
                                event.tx.send(SwarmHandlerResponse::Resp(swarm.behaviour_mut().req_resp.send_request(&peer, req))).ok();
                            },
                            SwarmHandlerRequest::Synchronise => {
                                sync(&mut swarm).await.ok();
                                event.tx.send(SwarmHandlerResponse::Synchronise).ok();
                            },
                            SwarmHandlerRequest::PeersCount => {
                                event.tx.send(SwarmHandlerResponse::PeersCount(swarm.connected_peers().count())).ok();
                            },
                            SwarmHandlerRequest::Peers => {
                                let mut vec = vec![];
                                swarm.connected_peers().for_each(|x| vec.push(x.clone()));
                                event.tx.send(SwarmHandlerResponse::Peers(vec)).ok();
                            },
                            SwarmHandlerRequest::Report(_, _, report) => {
                                let ser = NyumtReq(report);
                                let mut id;
                                {
                                    let swarm_set = swarm.behaviour();
                                    id = futures::executor::block_on(swarm_set.sett.default_master_node.read()).clone();
                                }
                                loop {
                                    match id {
                                        Some(master) => {
                                            info!("Sending log report to {}", master);
                                            swarm.behaviour_mut().req_resp.send_request(&master, ser);
                                            break;
                                        },
                                        None => {
                                            let peer = swarm.connected_peers().next();
                                            match peer {
                                                Some(p) => {
                                                    id = Some(p.clone());
                                                    *swarm.behaviour_mut().sett.default_master_node.write().await = Some(p.clone());
                                                },
                                                None    => break
                                            }
                                        }
                                    }
                                }
                                event.tx.send(SwarmHandlerResponse::Report(())).ok();
                            }
                        }
                    },
                    None => error!("Swarm event handler suddenly closed")
                }
            }
            event = swarm.select_next_some() => {
                if let libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } = event {
                    info!("Master node listening on {:?}", address);
                } else if let libp2p::swarm::SwarmEvent::ConnectionEstablished { peer_id, .. } = event {
                    info!("Connected to peer {}", peer_id);
                    let pk = swarm.behaviour().sett.pubkey.clone();
                    match PeerRequest::Hello(*PROTOCOL_VERSION, pk).try_to_vec() {
                        Ok(v) => {
                            swarm.behaviour_mut().req_resp.send_request(&peer_id, NyumtReq(v));
                        },
                        Err(e) => {
                            error!("Error serializing hello subscribe request: {}", e);
                            swarm.behaviour_mut().sett.stop.send(crate::Stop::Panic).await.ok();
                        }
                    };
                }
            }
        }
    };
}

pub async fn handler(swarm: Swarm<NyumtNodeProt>, swarm_rx: Receiver<SwarmHandlerReq>) -> Result<(), Box<dyn Error + Send + Sync>> {
    tokio::spawn(async move {
        swarm_event_handler(swarm, swarm_rx).await;
    });

    Ok(())
}

pub async fn sync(swarm: &mut Swarm<NyumtNodeProt>) -> Result<(), Box<dyn Error + Send + Sync>> {
    let swarm_set = swarm.behaviour().sett.clone(); 
    tokio::task::spawn_blocking(move || {
        let mut handle = swarm_set.swarm.clone();
        loop {
            if futures::executor::block_on(handle.peers()) > 0 {
                break;
            }
            futures::executor::block_on(tokio::time::sleep(*POLL_TIME));
        }
        futures::executor::block_on(async_sync(swarm_set, futures::executor::block_on(handle.peers_list())))
    });
    Ok(())
}

pub async fn async_sync(swarm: Arc<SwarmSettings>, list: Vec<PeerId>) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut seed: rand::rngs::StdRng = SeedableRng::from_entropy();
    let rand_peer = list.choose(&mut seed);

    match rand_peer {
        Some(v) => {
            // Now we get last config and master node list
            let mut handle = swarm.swarm.clone();

            let (tx, rx_master_list) = oneshot::channel();
            let req_id   = handle.get_master_node_list(v.clone()).await;
            swarm.resp.write().await.insert(req_id, tx);

            let (tx, rx_config) = oneshot::channel();
            let req_id   = handle.get_config(v.clone()).await;
            swarm.resp.write().await.insert(req_id, tx);

            let master_list = match rx_master_list.await {
                Ok(v) => match v {
                    PeerResponse::MasterNodeList(list) => {
                        let mut vec = vec![];
                        for i in list.0 {
                            vec.push((i.pubkey.try_to_vec()?, i.addr));
                        }
                        vec
                    },
                    _ => {
                        error!("Master node sending wrong response as master node list");
                        swarm.stop.send(crate::Stop::Panic).await.ok();
                        return Ok(());
                    }
                },
                Err(e) => {
                    error!("Failed to get initial master node list: {}", e);
                    swarm.stop.send(crate::Stop::Panic).await.ok();
                    return Ok(());
                }
            };
            
            let mut config = match rx_config.await {
                Ok(v) => match v {
                    PeerResponse::NodeConfig(conf) => conf,
                    _ => {
                        error!("Master node sending wrong response as config");
                        swarm.stop.send(crate::Stop::Panic).await.ok();
                        return Ok(());
                    }
                },
                Err(e) => {
                    error!("Failed to get initial config: {}", e);
                    swarm.stop.send(crate::Stop::Panic).await.ok();
                    return Ok(());
                }
            };

            let db = swarm.db.clone();
            cache::update_local_node_conf(&db, &config.try_to_vec()?).await;
            node::add_master_nodes(&db, &master_list).await;

            config.update();
            *swarm.conf.write().await = config;

            // TODO: Verify
            

            // Lastly we subscribe to get last updates
            handle.subscribe(v.clone()).await;
        },
        None => {
            error!("Default master node should be decided at this. Doing soft restart");
            swarm.stop.send(crate::Stop::SoftRestart).await.ok();
        }
    }

    Ok(())
}
