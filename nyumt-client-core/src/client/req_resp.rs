use std::{error::Error, sync::Arc};
use libp2p::{
    PeerId,
    request_response::{ResponseChannel, RequestId}
};
use borsh::{BorshDeserialize, BorshSerialize};
use nyumt_proto::{api::{ClientRequest, ClientResponse, ReqStat}, stats::Report};
use tokio::sync::RwLock;
use tracing::error;

use nyumt_proto::{
    types::{BodyData, Block},
    api::RequestBlock,
    database::{block, auth, node},
    keys::PublicKey,
    types::Permissions
};

use super::behavior::NyumtResp;
use crate::db::Database;

pub async fn send_response(resp: ClientResponse, swarm: &Arc<super::SwarmSettings>, resp_channel: ResponseChannel<super::behavior::NyumtResp>) {
    match resp.try_to_vec() {
        Ok(v)  => {
            swarm.swarm_handle.clone().respond(resp_channel, NyumtResp(v)).await;
        },
        Err(e) => {
            error!("Error serializing peer response: {}", e);
            swarm.swarm_handle.clone().respond(resp_channel, NyumtResp(ClientResponse::Error.try_to_vec().unwrap())).await;
        }
    }
}

pub async fn handle(request: &[u8], peer: PeerId, swarm: Arc<super::SwarmSettings>, resp_channel: ResponseChannel<super::behavior::NyumtResp>){
    let req = match ClientRequest::try_from_slice(request) {
        Ok(v)  => v,
        Err(_) => return ()
    };

    match req {
        ClientRequest::Block(blocks) => {
            send_response(ClientResponse::Block(ReqStat::Good), &swarm, resp_channel).await;
            for block in blocks {
                match handle_block(&swarm.db, &block, &swarm.publickey, swarm.last_height.clone()).await {
                    Ok(_) => (),
                    Err(e) => {
                        error!("Error while syncing: {}", e);
                        futures::executor::block_on(swarm.swarm_handle.clone().disconnect_from_peer(&peer)).ok();
                    },
                };
            }
        },
        ClientRequest::Stats(stats) => {
            send_response(ClientResponse::Stats(ReqStat::Good), &swarm, resp_channel).await;
            let db = swarm.db.clone();
            for report in stats.0 {
                match Report::try_from_slice(&report.data) {
                    Ok(rep) => {
                        node::store_stats_with_id(&db, report.id, &report.data, &report.sig, rep.timestamp).await.ok();
                    },
                    Err(e) => {
                        error!("Error serializing peer report: {}", e);
                    }
                }
            }
        },
        _ => ()
    }
}

pub async fn handle_resp(request: &[u8], peer: PeerId, swarm: Arc<super::SwarmSettings>, request_id: RequestId) {
    let req = match ClientResponse::try_from_slice(request) {
        Ok(v)  => v,
        Err(_) => return ()
    };
    match req {
        ClientResponse::IncompatibleVersion(min_ver) => {
            error!("Minimum supported version is {}, disconnecting", min_ver);
            swarm.swarm_handle.clone().disconnect_from_peer(&peer).await.ok();
        },
        ClientResponse::Request(status) | ClientResponse::GetStats(status) => {
            match swarm.resp.write().await.remove(&request_id) {
                Some(v) => {
                    v.send(status);
                },
                None => ()
            }
        },
        _ => ()
    }
}

pub async fn handle_block(db: &Database, meta: &RequestBlock, pk: &PublicKey, last: Arc<RwLock<i64>>) -> Result<(), Box<dyn Error>> {
    let block  = Block::try_from_slice(&meta.body)?;
    let body   = BodyData::try_from_slice(&block.body)?;
    let height = match i64::try_from(meta.id) {
        Ok(v)  => v,
        Err(_) => return Err(Box::new( std::io::Error::new( std::io::ErrorKind::Other, "Master node trying to overflow height" ) ))
    };

    if *last.read().await > height {
        return Err(Box::new( std::io::Error::new( std::io::ErrorKind::Other, "Master node is trying to rewrite older blocks" ) ));
    } else if meta.id != 0 {
        let ser_pk = match body.user_pk.try_to_vec() {
            Ok(v) => v,
            Err(e) => {
                return Err(Box::new(e));
            }
        };
        let user = match auth::get_user_with_pubkey(&db, ser_pk.as_slice()).await {
            Ok(v) => v,
            Err(e) => {
                return Err(e);
            }
        };
        let perms = match Permissions::try_from_slice(&user.permissions) {
            Ok(v) => v,
            Err(e) => {
                return Err(Box::new(e));
            }
        };
        let raw_req = match body.req.try_to_vec() {
            Ok(v) => v,
            Err(e) => return Err(Box::new(e))
        };
        match nyumt_proto::block::verify_req(&db.con, &raw_req, &body.req, &body.user_pk, &perms, &body.sign).await {
            Ok(v) => {
                match v {
                    Some(v) => {
                        return Err(Box::new( std::io::Error::new( std::io::ErrorKind::Other, format!("Block {} is contradictory: {:?}", meta.id, v) ) ));
                    },
                    None => ()
                }
            },
            Err(e) => return Err(e)
        }
    }
    match block::add_new(&db, meta.id, &meta.body, &block.hash).await {
        Ok(_) => (),
        Err(e) => {
            return Err(e);
        }
    }
    match nyumt_proto::db::interpret_block(pk, &db.con, &body.req).await {
        Ok(_) => (),
        Err(e) => {
            return Err(e);
        }
    };
    
    *last.write().await += 1;
    Ok(())
}
