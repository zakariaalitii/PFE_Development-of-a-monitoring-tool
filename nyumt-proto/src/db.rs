use sqlx::{sqlite::Sqlite, Pool, Transaction};
use std::error::Error;
use borsh::BorshSerialize;
use tracing::info;

use crate::{types::{ RequestType, Order }, keys::PublicKey};
use crate::config::{MasterConfig, NodeConfig};

pub enum Update {
    ConfigUpdate(String, MasterConfig, NodeConfig),
    NodeConfigChange(PublicKey, String),
    MasterNodeListUpdate,
    None
}

pub async fn interpret_block(pk: &PublicKey, db: &Pool<Sqlite>, reqs: &[RequestType]) -> Result<Vec<Update>, Box<dyn Error>> {
    let mut tx      = db.begin().await?;
    let mut updates = vec![];
    for req in reqs {
        match req {
            RequestType::Order(o) => updates.push(interpret_order(pk, &mut tx, o).await?)
        }
    }
    tx.commit().await?;
    Ok(updates)
}

async fn interpret_order<'a>(pk: &PublicKey, tx: &mut Transaction<'a, Sqlite>, order: &Order) -> Result<Update, Box<dyn Error>> {
    let mut update = Update::None;
    match order {
        Order::AddUser { name, publickey, perm } => {
            info!("New user {} with public key {} added", name, publickey);
            sqlx::query("insert into users (name, public_key, permissions, active) values (?1, ?2, ?3, true)")
                .bind(name)
                .bind(publickey.try_to_vec()?)
                .bind(perm.try_to_vec()?)
                .execute(tx)
                .await?;
        },
        Order::AddMasterNode { name, publickey } => {
            info!("New master node {} with public key {} added", name, publickey);
            sqlx::query("insert into node (name, public_key, master_node, vote_weight, conf) values (?1, ?2, true, 1, \"default\")")
                .bind(name)
                .bind(publickey.try_to_vec()?)
                .execute(tx)
                .await?;
            update = Update::MasterNodeListUpdate;
        },
        Order::AddNode { name, publickey } => {
            info!("New node {} with public key {} added", name, publickey);
            sqlx::query("insert into node (name, public_key, master_node, conf) values (?1, ?2, false, \"default\")")
                .bind(name)
                .bind(publickey.try_to_vec()?)
                .execute(tx)
                .await?;
        },
        Order::EditUser { name, publickey, perm } => {
            info!("User with public key {} info changed", publickey);
            sqlx::query("update users set name = ?1, permissions = ?2 where public_key = ?3")
                .bind(name)
                .bind(perm.try_to_vec()?)
                .bind(publickey.try_to_vec()?)
                .execute(tx)
                .await?;
        },
        Order::EditMasterNode { name, publickey } => {
            info!("Master node with public key {} info changed", publickey);
            sqlx::query("update node set name = ?1 where public_key = ?2 and master_node = true")
                .bind(name)
                .bind(publickey.try_to_vec()?)
                .execute(tx)
                .await?;
            update = Update::MasterNodeListUpdate;
        },
        Order::EditNode { name, publickey } => {
            info!("Node with public key {} info changed", publickey);
            sqlx::query("update node set name = ?1 where public_key = ?2")
                .bind(name)
                .bind(publickey.try_to_vec()?)
                .execute(tx)
                .await?;
        },
        Order::UpdatePublicKey { publickey, new_publickey } => {
            sqlx::query("update node set publickey = ?1 where public_key = ?2;\
                         update users set publickey = ?1 where public_key = ?2")
                .bind(new_publickey.try_to_vec()?)
                .bind(publickey.try_to_vec()?)
                .execute(tx)
                .await?;
        },
        Order::UpdateVoteWeight { publickey, weight } => {
            sqlx::query("update node set vote_weight = ?1 where public_key = ?2 and master_node = true")
                .bind(weight)
                .bind(publickey.try_to_vec()?)
                .execute(tx)
                .await?;
        }
        Order::RemoveUser { publickey } => {
            sqlx::query("update users set active = false where public_key = ?1")
                .bind(publickey.try_to_vec()?)
                .execute(tx)
                .await?;
        }
        e @ (Order::RemoveMasterNode { publickey } | Order::RemoveNode { publickey }) => {
            // This won't leave a trace of this node besides the chain
            sqlx::query("delete from node where public_key = ?1")
                .bind(publickey.try_to_vec()?)
                .execute(tx)
                .await?;
            match e {
                Order::RemoveMasterNode { .. } => update = Update::MasterNodeListUpdate,
                _ => ()
            }
        },
        Order::SetConfig { name, master, node } => {
            info!("Node Config with name {} configured", name);
            sqlx::query("insert into config (name, master_conf, node_conf) values (?1, ?2, ?3) on conflict(name) do update set master_conf = ?2, node_conf = ?3 \
                         where name = ?1")
                .bind(name)
                .bind(master.try_to_vec()?)
                .bind(node.try_to_vec()?)
                .execute(tx)
                .await?;
            update = Update::ConfigUpdate(name.clone(), master.clone(), node.clone());
        },
        Order::RemoveConfig { name } => {
            sqlx::query("delete from config where name = ?1")
                .bind(name)
                .execute(tx)
                .await?;
        },
        Order::ChangeNodeConfig { publickey, name } => {
            sqlx::query("update node set conf = ?1 where public_key = ?2")
                .bind(name)
                .bind(publickey.try_to_vec()?)
                .execute(tx)
                .await?;
            update = Update::NodeConfigChange(publickey.clone(), name.clone());
        },
    }

    Ok(update)
}
