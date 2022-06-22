use std::{error::Error, sync::Arc};
use tokio::sync::RwLock;
use borsh::BorshSerialize;
use nyumt_proto::{
    api::Request,
    types::{RequestType, Order, Permissions, Permission},
    keys::PublicKey,
    database::{Database, node}, config::{MasterConfig, NodeConfig}
};
use libp2p::identity;

use crate::storage::{KeyPair, Storage};

pub async fn parse_perms(permissions: &[String]) -> Result<Permissions, Box<dyn Error>> {
    let mut perms = vec![];
    for perm in permissions {
        match perm.as_str() {
            "root" => perms.push(Permission::Root),
            "monitor" => perms.push(Permission::Monitor),
            _ => return Err(Box::new( std::io::Error::new( std::io::ErrorKind::Other, format!("Unknewn permission {}", perm) ) ))
        }
    }
    Ok(Permissions(perms))
}

pub async fn sign_order(storage: Arc<RwLock<Storage>>, session_id: usize, order: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let sig;
    match &storage.read().await.get().session.0[session_id].keys {
        KeyPair::ED25519(k) => {
            let pk  = identity::Keypair::Ed25519(identity::ed25519::Keypair::decode(k.0.clone().as_mut())?);
            sig     = pk.sign(order)?;
        }
    };

    Ok(sig)
}

pub async fn pack_order(storage: Arc<RwLock<Storage>>, session_id: usize, order: Vec<RequestType>) -> Result<Request, Box<dyn Error>> {
    let sig;
    {
        let ser_order = order.try_to_vec()?;
        sig           = sign_order(storage, session_id, &ser_order).await?;
    }

    Ok(Request {
        req: order,
        sign: sig
    })
}

pub async fn add_user(storage: Arc<RwLock<Storage>>, session_id: usize, pk: &str, name: &str, permissions: &[String]) -> Result<Request, Box<dyn Error>> {
    let pubkey = PublicKey::parse_str(pk)?;
    let perms  = parse_perms(permissions).await?;
    let order  = vec![RequestType::Order(Order::AddUser { name: name.to_owned(), publickey: pubkey, perm: perms })];
    pack_order(storage, session_id, order).await
}

pub async fn update_user(storage: Arc<RwLock<Storage>>, session_id: usize, pk: &str, name: &str, permissions: &[String]) -> Result<Request, Box<dyn Error>> {
    let pubkey = PublicKey::parse_str(pk)?;
    let perms  = parse_perms(permissions).await?;
    let order  = vec![RequestType::Order(Order::EditUser { name: name.to_owned(), publickey: pubkey, perm: perms })];
    pack_order(storage, session_id, order).await
}

pub async fn remove_user(storage: Arc<RwLock<Storage>>, session_id: usize, pk: &str) -> Result<Request, Box<dyn Error>> {
    let pubkey = PublicKey::parse_str(pk)?;
    let order  = vec![RequestType::Order(Order::RemoveUser { publickey: pubkey })];
    pack_order(storage, session_id, order).await
}

pub async fn add_node(storage: Arc<RwLock<Storage>>, session_id: usize, pk: &str, name: &str, master: bool) -> Result<Request, Box<dyn Error>> {
    let pubkey = PublicKey::parse_str(pk)?;
    let order;
    if master {
        order  = vec![RequestType::Order(Order::AddMasterNode { name: name.to_owned(), publickey: pubkey })];
    } else {
        order  = vec![RequestType::Order(Order::AddNode { name: name.to_owned(), publickey: pubkey })];
    }
    pack_order(storage, session_id, order).await
}

pub async fn update_node(db: &Database, storage: Arc<RwLock<Storage>>, session_id: usize, pk: &str, name: &str) -> Result<Request, Box<dyn Error>> {
    let pubkey = PublicKey::parse_str(pk)?;
    let order;
    if node::is_node_master(db, &pubkey.try_to_vec()?).await? {
        order = vec![RequestType::Order(Order::EditMasterNode { name: name.to_owned(), publickey: pubkey })];
    } else {
        order = vec![RequestType::Order(Order::EditNode { name: name.to_owned(), publickey: pubkey })];
    }
    pack_order(storage, session_id, order).await
}

pub async fn remove_node(db: &Database, storage: Arc<RwLock<Storage>>, session_id: usize, pk: &str) -> Result<Request, Box<dyn Error>> {
    let pubkey = PublicKey::parse_str(pk)?;
    let order;
    if node::is_node_master(db, &pubkey.try_to_vec()?).await? {
        order = vec![RequestType::Order(Order::RemoveMasterNode { publickey: pubkey })];
    } else {
        order = vec![RequestType::Order(Order::RemoveNode { publickey: pubkey })];
    }
    pack_order(storage, session_id, order).await
}

pub async fn change_node_config(storage: Arc<RwLock<Storage>>, session_id: usize, pk: &str, name: &str) -> Result<Request, Box<dyn Error>> {
    let pubkey = PublicKey::parse_str(pk)?;

    let order  = vec![RequestType::Order(Order::ChangeNodeConfig { publickey: pubkey, name: name.to_owned() })];
    pack_order(storage, session_id, order).await
}

pub async fn set_config(storage: Arc<RwLock<Storage>>, session_id: usize, name: &str, master: &MasterConfig, node: &NodeConfig) -> Result<Request, Box<dyn Error>> {
    let order  = vec![RequestType::Order(Order::SetConfig { name: name.to_owned(), master: master.to_owned(), node: node.to_owned() })];
    pack_order(storage, session_id, order).await
}

pub async fn remove_config(storage: Arc<RwLock<Storage>>, session_id: usize, name: &str) -> Result<Request, Box<dyn Error>> {
    let order  = vec![RequestType::Order(Order::RemoveConfig { name: name.to_owned() })];
    pack_order(storage, session_id, order).await
}
