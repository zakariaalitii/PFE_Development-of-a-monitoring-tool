use std::error::Error;
use std::time::SystemTime;
use tracing::error;

use crate::master::{
    types,
    activity
};
use crate::utils::keys;
use crate::database::{Database, block};
use nyumt_proto::db as db_interpret;

pub fn create(db: &Database, node_keypair: &keys::LocalPeerKey, pk: &keys::PublicKey) -> Result<(), Box<dyn Error>> {
    let block = types::BodyData {
        req: vec![
            types::RequestType::Order(
                types::Order::AddUser { 
                    name: "root".to_owned(),
                    publickey: pk.clone(),
                    perm: types::Permissions(vec![types::Permission::Root])
            }),
            types::RequestType::Order(
                types::Order::AddMasterNode {
                    name: "controller0".to_owned(),
                    publickey: node_keypair.public()
            }),
            types::RequestType::Order(
                types::Order::SetConfig {
                    name: "default".to_owned(),
                    master: crate::config::MasterConfig::default(),
                    node: crate::config::NodeConfig::default()
                }
            )
        ],
        sign: vec![],
        user_pk: pk.clone(),
        timestamp: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs(),
    };

    let (bytes, hash) = activity::serialize_block(&block, None, None)?;

    match futures::executor::block_on(block::add_new(db, 0, bytes.as_slice(), &hash)) {
        Ok(_)  => (),
        Err(e) => error!("Error occured while writing new block {}", e)
    }
    match futures::executor::block_on(db_interpret::interpret_block(&node_keypair.public().clone(), &db.con, &block.req)) {
        Ok(_)  => (),
        Err(e) => {
            error!("Error saving updates to database: {}", e);
        }
    }

    Ok(())
}
