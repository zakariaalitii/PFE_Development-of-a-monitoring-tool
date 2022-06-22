use std::sync::Arc;

use libp2p::{
    PeerId,
    request_response::ResponseChannel
};
use borsh::{BorshDeserialize, BorshSerialize};
use nyumt_proto::{
    api::{ClientRequest, ClientResponse, RequestResp, GetStats},
    block::verify_req,
    PROTOCOL_VERSION,
    types::{Permission, BodyData, RequestStatus}
};
use tracing::{info, error};
use tokio::sync::oneshot;
use chrono::Utc;
use futures::executor::block_on;

use crate::master_serve::activity;
use crate::master::types::Request;

use super::behavior::NyumtResp;

pub async fn send_response(resp: ClientResponse, swarm: &Arc<super::SwarmSettings>, resp_channel: ResponseChannel<super::behavior::NyumtResp>) {
    match resp.try_to_vec() {
        Ok(v)  => {
            swarm.swarm_handle.clone().respond(resp_channel, NyumtResp(v)).await;
        },
        Err(e) => {
            error!("Node cannot serialize responses for communications, stopping: {}", e);
            swarm.swarm_handle.clone().respond(resp_channel, NyumtResp(ClientResponse::Error.try_to_vec().unwrap())).await;
        }
    }
}

pub async fn handle<'a>(request: &[u8], peer: PeerId, swarm: Arc<super::SwarmSettings>, resp_channel: ResponseChannel<super::behavior::NyumtResp>){
    let req = match ClientRequest::try_from_slice(request) {
        Ok(v)  => v,
        Err(_) => return ()
    };
    match req {
        ClientRequest::Hello(version, pk, settings) => {
            let valid_pk = match pk.to_publickey() {
                Ok(v) => {
                    match peer.is_public_key(&v) {
                        Some(v) => v,
                        None => false
                    }
                },
                Err(_) => false
            };

            if version != *PROTOCOL_VERSION && valid_pk {
                // In order to keep our network communication understandable
                // we only accept nodes with same version
                send_response(ClientResponse::Error, &swarm, resp_channel).await;
                swarm.swarm_handle.clone().disconnect_from_peer(&peer).await.ok();
                return;
            }
            info!("Got handshake from client {} (public key: {})", peer, pk);
            send_response(ClientResponse::Hi(*PROTOCOL_VERSION), &swarm, resp_channel).await;
        
            let swarm_handle = swarm.swarm_handle.clone();
            let db           = swarm.db.clone();
            let auth         = swarm.auth.clone();
            // Now we create channel for events
            tokio::task::spawn_blocking(move || {
                futures::executor::block_on(activity::event_handler(swarm_handle, db, auth, peer, pk, settings))
            });
        },
        ClientRequest::Request(req) => {
            if verify_user_permissions(&peer, &[], &swarm).await {
                let user;
                match swarm.auth.read().await.get(&peer) {
                    Some(_user) => user = Some((_user.pubkey.to_owned(), _user.permissions.clone())),
                    None    => user = None
                }

                let db         = swarm.db.clone();
                let req_sender = swarm.req_sender.clone();
                let ret        = match tokio::task::spawn_blocking(move || {
                    if user.is_some() {
                        let user = user.unwrap();
                        match req.req.try_to_vec() {
                            Ok(v) => {
                                let resp = match block_on(verify_req(&db.con, &v, &req.req, &user.0, &user.1, &req.sign)) {
                                    Ok(Some(err)) => Some(RequestResp::RequestError(err)),
                                    Ok(None) => {
                                        let (one_tx, rx) = oneshot::channel();
                                        match block_on(req_sender.send(Request {
                                            body: crate::master::types::ChannelRequestType::Block(BodyData {
                                                req: req.req,
                                                sign: req.sign,
                                                user_pk: user.0,
                                                timestamp: Utc::now().timestamp() as u64
                                            }),
                                            resp_channel: one_tx
                                        })) {
                                            Ok(_)  => {
                                                match block_on(rx) {
                                                    Ok(v)  => Some(RequestResp::RequestStatus(v)),
                                                    Err(e) => {
                                                        error!("Error getting response: {}", e);
                                                        None
                                                    }
                                                }
                                            },
                                            Err(e) => {
                                                error!("Error using request channel: {}", e);
                                                None
                                            }
                                        }
                                    },
                                    _ => None
                                };

                                match resp {
                                    Some(v) => return v,
                                    None => ()
                                }
                            },
                            Err(e) => error!("Error serializing request: {}", e)
                        }
                    }
                    RequestResp::RequestStatus(RequestStatus::NodeError)
                }).await {
                    Ok(v) => v,
                    Err(_) => RequestResp::RequestStatus(RequestStatus::NodeError)
                };
                send_response(ClientResponse::Request(ret), &swarm, resp_channel).await;
            }
        },
        ClientRequest::GetStats(pk, oid) => {
            tokio::task::spawn_blocking(move || {
                match pk.to_publickey() {
                    Ok(v) => {
                        let peer_id = v.to_peer_id();

                        let (one_tx, rx) = oneshot::channel();
                        match block_on(swarm.req_sender.send(Request {
                            body: crate::master::types::ChannelRequestType::GetStats(peer_id, oid),
                            resp_channel: one_tx
                        })) {
                            Ok(_) => {
                                match block_on(rx) {
                                    Ok(v) => {
                                        block_on(send_response(ClientResponse::GetStats(RequestResp::RequestStatus(v)), &swarm, resp_channel));
                                        return;
                                    },
                                    Err(_) => ()
                                }
                            },
                            Err(e) => error!("Error using request channel: {}", e)
                        }
                    },
                    _ => ()
                }
                block_on(send_response(ClientResponse::Error, &swarm, resp_channel));
            });
        },
        _ => {
            send_response(ClientResponse::Error, &swarm, resp_channel).await;
        }
    }
}

async fn verify_user_permissions(peer: &PeerId, permissions: &[Permission], swarm: &Arc<super::SwarmSettings>) -> bool {
    // verify if user has enough permission
    match swarm.auth.read().await.get(peer) {
        Some(v) => {
            if permissions.is_empty() {
                return true;
            }
            for perm in permissions {
                if v.permissions.0.contains(perm) {
                    return true;
                }
            }
            return false;
        },
        None => ()
    }
    false
}
