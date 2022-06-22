use std::sync::Arc;
use libp2p::{
    PeerId,
    request_response::{ResponseChannel, RequestId}
};
use borsh::{BorshDeserialize, BorshSerialize};
use nyumt_proto::{
    peer::{ PeerRequest, PeerResponse },
    stats::{generate_report, DbStruct, System, Platform},
};
use tracing::{error, info};

use crate::node::net_behavior::NyumtResp;
use crate::master::{self, req_resp};
use crate::database::{node, cache};

pub enum NodeLevel {
    Normal(Arc<super::SwarmSettings>, ResponseChannel<super::net_behavior::NyumtResp>),
    Master(Arc<master::SwarmSettings>, ResponseChannel<master::net_behavior::NyumtResp>)
}

pub enum NodeLevelResponse {
    Normal(Arc<super::SwarmSettings>),
    Master(Arc<master::SwarmSettings>)
}

pub async fn send_response(resp: PeerResponse, swarm: &Arc<super::SwarmSettings>, resp_channel: ResponseChannel<super::net_behavior::NyumtResp>) {
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

pub async fn handle(request: &[u8], peer: PeerId, node_type: NodeLevel) {
    let req = match PeerRequest::try_from_slice(request) {
        Ok(v)  => v,
        Err(_) => return ()
    };
    match node_type {
        NodeLevel::Master(swarm, resp_channel) => req_resp::handle(&req, peer, swarm, resp_channel).await,
        NodeLevel::Normal(swarm, resp_channel) => {
            match req {
                PeerRequest::UpdateConfig(mut config) => {
                    info!("Got update config request from: {}", peer);
                    let db = swarm.db.clone();
                    match config.try_to_vec() {
                        Ok(v) => {
                            cache::update_local_node_conf(&db, &v).await.ok();
                        },
                        Err(e) => error!("Error updating node config: {}", e)
                    }
                    config.update();
                    *swarm.conf.write().await = config;
                    send_response(PeerResponse::Good, &swarm, resp_channel).await
                },
                PeerRequest::UpdateMasterNodeList(list) => {
                    info!("Got update master node list request from: {}", peer);
                    let db = swarm.db.clone();
                    let mut vec = vec![];
                    for i in list.0 {
                        vec.push((match i.pubkey.try_to_vec() {
                            Ok(v) => v,
                            Err(e) => {
                                error!("Error updating node master nodes list: {}", e);
                                send_response(PeerResponse::Error, &swarm, resp_channel).await;
                                return;
                            }
                        }, i.addr));
                    }
                    node::add_master_nodes(&db, &vec).await.ok();
                    send_response(PeerResponse::Good, &swarm, resp_channel).await
                },
                PeerRequest::GetStats(oid) => {
                    let sys       = Arc::new(System::new());
                    let db_struct = Arc::new(DbStruct::from_oid(&oid));

                    match generate_report(&sys, &db_struct).await {
                        Ok(rep) => {
                            match rep.try_to_vec() {
                                Ok(v) => {
                                    match swarm.keypair.sign(&v) {
                                        Ok(v) => {
                                            send_response(PeerResponse::GetStats(rep, v), &swarm, resp_channel).await;
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
                    send_response(PeerResponse::Error, &swarm, resp_channel).await;
                },
                _ => send_response(PeerResponse::Error, &swarm, resp_channel).await
            }
        }
    }
}

pub async fn handle_response(response: &[u8], peer: PeerId, request_id: RequestId, node_type: NodeLevelResponse) {
    let resp = match PeerResponse::try_from_slice(response) {
        Ok(v)  => v,
        Err(_) => return ()
    };
    match node_type {
        NodeLevelResponse::Normal(swarm_set) => {
            match resp {
                PeerResponse::Hi(_) => (),
                PeerResponse::NodeConfigHash(_) | PeerResponse::NodeConfig(_) | PeerResponse::MasterNodeListHash(_) | PeerResponse::MasterNodeList(_) => {
                    match swarm_set.resp.write().await.remove(&request_id) {
                        Some(v) => {
                            v.send(resp).ok();
                        },
                        None    => ()
                    }
                },
                _ => ()
            }
        },
        NodeLevelResponse::Master(swarm_set) => req_resp::handle_response(resp, peer, request_id, swarm_set).await
    }
}
