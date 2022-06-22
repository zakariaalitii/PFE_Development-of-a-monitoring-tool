use std::sync::Arc;
use futures::StreamExt;
use libp2p::{PeerId, request_response::{ResponseChannel, RequestId}};
use nyumt_proto::{
    PROTOCOL_VERSION,
    peer::{PeerRequest, PeerResponse, RequestStatus, MasterNode, MasterNodes},
    stats::Report, config::NodeConfig
};
use tracing::{error, info};
use borsh::{BorshSerialize, BorshDeserialize};

use crate::master::{SwarmSettings, net_behavior::NyumtResp};
use crate::database::{block, node};
use crate::utils::keys::PublicKey;

pub async fn send_response(resp: PeerResponse, swarm: &Arc<SwarmSettings>, resp_channel: ResponseChannel<super::net_behavior::NyumtResp>) {
    match resp.try_to_vec() {
        Ok(v)  => {
            swarm.swarm.clone().respond(resp_channel, NyumtResp(v)).await;
        },
        Err(e) => {
            error!("Node cannot serialize responses for communications, stopping: {}", e);
            swarm.stop.send(crate::Stop::Panic).await.ok();
        }
    }
}

pub async fn handle(req: &PeerRequest, peer: PeerId, swarm: Arc<SwarmSettings>, resp_channel: ResponseChannel<super::net_behavior::NyumtResp>) {
    match req {
        PeerRequest::Hello(version, pk) => {
            info!("Got handshake from node {} (public key: {})", peer, pk);
            send_response(PeerResponse::Hi(*PROTOCOL_VERSION), &swarm, resp_channel).await;
            if *version != *PROTOCOL_VERSION {
                // In order to keep our network communication understandable
                // we only accept nodes with same version
                swarm.swarm.clone().disconnect_from_peer(&peer).await.ok();
            } else {
                let db = swarm.db.clone();
                match pk.try_to_vec() {
                    Ok(v) => {
                        node::update_peerid(&db, &v, peer.to_base58()).await.ok();
                    },
                    Err(e) => error!("Peer {} sent invalid public key: {}", peer, e)
                }
            }
        },
        PeerRequest::HelloSubscribe => {
            info!("New subscribed node {}", peer);
            let db   = swarm.db.clone();
            let conf = match node::get_peer_conf_name(&db, &peer.to_base58()).await {
                Ok(v) => v,
                Err(e) => {
                    error!("{}", e);
                    send_response(PeerResponse::Error, &swarm, resp_channel).await;
                    return
                }
            };
            let mut lock = swarm.subscribed_nodes.write().await;
            if !lock.contains_key(&conf) {
                let mut vec = Vec::new();
                vec.push(peer);
                lock.insert(conf, vec);
            } else {
                lock.get_mut(&conf).unwrap().push(peer);
            }
            send_response(PeerResponse::Good, &swarm, resp_channel).await;
        },
        PeerRequest::UnsubscribeDefaultNode => {
            let db   = swarm.db.clone();
            let conf = match node::get_peer_conf_name(&db, &peer.to_base58()).await {
                Ok(v) => v,
                Err(_) => return
            };
            let mut lock = swarm.subscribed_nodes.write().await;
            if lock.contains_key(&conf) {
                lock.get_mut(&conf).unwrap().retain(|x| *x != peer);
            }
        },
        PeerRequest::BlockHeight => {
            let db = swarm.db.clone();
            match block::get_last_block_height(&db).await {
                Ok(Some(height)) => send_response(PeerResponse::BlockHeight(height), &swarm, resp_channel).await,
                _                => {
                    send_response(PeerResponse::Error, &swarm, resp_channel).await;
                }
            }
        },
        PeerRequest::Report(report) => {
            info!("Got log report from {}", peer);
            match Report::try_from_slice(&report.report) {
                Ok(rep) => {
                    let db      = swarm.db.clone();
                    match node::get_peer_pk(&db, &peer.to_base58()).await {
                        Ok(Some(node_pk)) => {
                            node::store_stats(&db, &node_pk, &report.report, &report.sig, rep.timestamp).await.ok();
                            send_response(PeerResponse::Report(RequestStatus::Good), &swarm, resp_channel).await;
                        },
                        _ => {
                            send_response(PeerResponse::Report(RequestStatus::Bad), &swarm, resp_channel).await;
                        }
                    }
                },
                Err(e) => {
                    error!("Error serializing peer report: {}", e);
                    send_response(PeerResponse::Report(RequestStatus::Bad), &swarm, resp_channel).await;
                }
            }
        }
        e @ (PeerRequest::NodeConfig | PeerRequest::NodeConfigHash) => {
            info!("Node config request from {}", peer);
            let db = swarm.db.clone();
            match node::get_peer_conf(&db, &peer.to_base58()).await {
                Ok(v) => {
                match NodeConfig::try_from_slice(&v) {
                        Ok(mut conf) => {
                            match e {
                                PeerRequest::NodeConfig => {
                                    send_response(PeerResponse::NodeConfig(conf), &swarm, resp_channel).await
                                },
                                PeerRequest::NodeConfigHash => {
                                    conf.update();
                                    send_response(PeerResponse::NodeConfigHash(conf.hash), &swarm, resp_channel).await;
                                },
                                _ => ()
                            }
                            return;
                        },
                        Err(e) => error!("Error serializing node config: {}", e)
                    }
                },
                Err(e) => {
                    error!("Error getting peer config: {}", e);
                }
            }
            send_response(PeerResponse::Error, &swarm, resp_channel).await;
        },
        req_type @ (PeerRequest::MasterNodeList | PeerRequest::MasterNodeListHash) => {
            info!("Master node list request from {}", peer);
            let db       = swarm.db.clone();
            let mut list = vec![];
            let mut iter = node::get_master_nodes(&db).await;
            while let Some(mnode) = iter.next().await {
                match mnode {
                    Ok(v) => {
                        let mpk = match PublicKey::try_from_slice(&v.public_key) {
                            Ok(_pk) => _pk,
                            Err(e) => {
                                error!("Error serializing master node public key: {}", e);
                                send_response(PeerResponse::Error, &swarm, resp_channel).await;
                                return;
                            }
                        };
                        list.push(MasterNode {
                            addr: if mpk == swarm.pubkey { Some(swarm.addr.read().await.clone().unwrap().to_string()) } else { None },
                            pubkey: mpk
                        });
                    },
                    Err(e) => {
                        error!("Error fetching master node list from database: {}", e);
                        send_response(PeerResponse::Error, &swarm, resp_channel).await;
                        return;
                    }
                }
            }
            match req_type {
                PeerRequest::MasterNodeList => send_response(PeerResponse::MasterNodeList(MasterNodes(list)), &swarm, resp_channel).await,
                _                           => {
                    match MasterNodes(list).hash() {
                        Ok(v) => futures::executor::block_on(send_response(PeerResponse::MasterNodeListHash(v), &swarm, resp_channel)),
                        Err(e) => {
                            error!("Error calculating hash for master node list: {}", e);
                            futures::executor::block_on(send_response(PeerResponse::Error, &swarm, resp_channel));
                            return;
                        }
                    }
                }
            }
        },
        _ => send_response(PeerResponse::Error, &swarm, resp_channel).await
    }
}

pub async fn handle_response(response: PeerResponse, _peer: PeerId, request_id: RequestId, swarm: Arc<SwarmSettings>) {
    match swarm.resp.write().await.remove(&request_id) {
        Some(v) => {
            v.send(response).ok();
        },
        None    => ()
    }
}

