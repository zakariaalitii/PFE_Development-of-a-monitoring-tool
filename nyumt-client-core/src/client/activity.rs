use std::error::Error;
use libp2p::{Swarm, PeerId, request_response::ResponseChannel};
use tokio::sync::{
    mpsc::{Receiver, Sender},
    oneshot,
};
use borsh::BorshSerialize;
use tracing::error;
use futures::StreamExt;
use nyumt_proto::{
    api::{ClientResponse, RequestResp, ClientRequest, SessionSettings},
    PROTOCOL_VERSION,
    keys::PublicKey
};

use super::behavior::{NyumtReq, NyumtClientProt};


#[derive(Debug, Clone)]
pub struct SwarmHandlerCall {
    pub tx: Sender<SwarmHandlerReq>
}

impl SwarmHandlerCall {
    pub async fn hello(&mut self, peer: &PeerId, pk: &PublicKey, last_height: i64, last_timestamp: u64) -> Result<(), ()> {
        let req = match ClientRequest::Hello(*PROTOCOL_VERSION, pk.clone(), SessionSettings { last_request_height: last_height, last_timestamp }).try_to_vec() {
            Ok(v) => v,
            Err(e) => {
                error!("Error serializing hello request: {}", e);
                return Err(());
            }
        };

        match self.call(SwarmHandlerRequest::Hello(peer.clone(), req)).await {
            SwarmHandlerResponse::Hello => Ok(()),
            _ => {
                panic!("Wrong Implementation in swarm channel");
            }
        }
    }

    pub async fn send_order(&mut self, peer: &PeerId, order: Vec<u8>) -> Result<oneshot::Receiver<RequestResp>, ()> {
        match self.call(SwarmHandlerRequest::SendOrder(peer.clone(), order)).await {
            SwarmHandlerResponse::SendOrder(v) => Ok(v),
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

    pub async fn respond(&mut self, channel: ResponseChannel<super::behavior::NyumtResp>, data: super::behavior::NyumtResp) -> () {
        match self.call(SwarmHandlerRequest::Response(channel, data)).await {
            SwarmHandlerResponse::None => (),
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
    Hello,
    SendOrder(oneshot::Receiver<RequestResp>),
    DisconnectPeerStatus(Result<(), ()>),
    Error,
    None
}

pub struct SwarmHandlerReq {
    req: SwarmHandlerRequest,
    tx: oneshot::Sender<SwarmHandlerResponse>
}

enum SwarmHandlerRequest {
    Hello(PeerId, Vec<u8>),
    SendOrder(PeerId, Vec<u8>),
    DisconnectPeer(PeerId),
    Error(PeerId),
    Response(ResponseChannel<super::behavior::NyumtResp>, super::behavior::NyumtResp)
}

pub async fn swarm_event_handler(mut swarm: Swarm<NyumtClientProt>, mut rx: Receiver<SwarmHandlerReq>) {
    let mut is_close = swarm.behaviour_mut().sett.close_connection.write().await.take().unwrap();
    let close        = swarm.behaviour_mut().sett.disconnected.write().await.take().unwrap();
    loop {
        tokio::select! {
            _   = &mut is_close => {
                break
            },
            rec = rx.recv() => {
                match rec {
                    Some(event) => {
                        match event.req {
                            SwarmHandlerRequest::Response(ch, resp) => {
                                swarm.behaviour_mut().req_resp.send_response(ch, resp).ok();
                                event.tx.send(SwarmHandlerResponse::None).ok();
                            },
                            SwarmHandlerRequest::Hello(peer_id, hello) => {
                                swarm.behaviour_mut().req_resp.send_request(&peer_id, NyumtReq(hello));
                                event.tx.send(SwarmHandlerResponse::Hello).ok();
                            },
                            SwarmHandlerRequest::SendOrder(peer_id, order) => {
                                let req_id = swarm.behaviour_mut().req_resp.send_request(&peer_id, NyumtReq(order));
                                let (tx, rx) = oneshot::channel();
                                swarm.behaviour_mut().sett.resp.write().await.insert(req_id, tx);
                                event.tx.send(SwarmHandlerResponse::SendOrder(rx)).ok();
                            },
                            SwarmHandlerRequest::DisconnectPeer(peer_id) => {
                                event.tx.send(SwarmHandlerResponse::DisconnectPeerStatus(swarm.disconnect_peer_id(peer_id))).ok();
                            }
                            SwarmHandlerRequest::Error(peer_id) => {
                                match ClientResponse::Error.try_to_vec() {
                                    Ok(v) => {
                                        swarm.behaviour_mut().req_resp.send_request(&peer_id, NyumtReq(v));
                                    },
                                    Err(_) => {
                                        error!("Error serializing simple error, closing connection");
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
                    libp2p::swarm::SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        swarm.behaviour_mut().sett.peer.write().await.insert(peer_id);
                    },
                    libp2p::swarm::SwarmEvent::ConnectionClosed { .. } => {
                        close.send(()).ok();
                        break;
                    }
                    _ => ()
                }
            }
        }
    };
}

pub async fn handler(swarm: Swarm<NyumtClientProt>, swarm_rx: Receiver<SwarmHandlerReq>) -> Result<(), Box<dyn Error + Send + Sync>> {

    swarm_event_handler(swarm, swarm_rx).await;

    Ok(())
}
