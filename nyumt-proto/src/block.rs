use std::error::Error;
use borsh::BorshSerialize;
use sha2::{ Digest, Sha256 };
use sqlx::{Pool, Sqlite};

use crate::keys::PublicKey;
use crate::types::{ RequestType, RequestError, Order, Permission, Permissions };

pub async fn verify_hash(raw_req: &[u8], hash: &[u8]) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(raw_req);
    hasher.finalize().to_vec().eq(hash)
}

/// Verifying if req is valid, returns type of invalid type if request is malformed and None when
/// everything is good
pub async fn verify_req(db: &Pool<Sqlite>, raw_req: &[u8], req: &[RequestType], user_pk: &PublicKey, perms: &Permissions, sig: &[u8]) -> Result<Option<RequestError>, Box<dyn Error + Send + Sync>> {
    if user_pk.verify_sig(raw_req, sig)? {
        for r in req {
            match &r {
                RequestType::Order(ord) => {
                    let ver = verify_order(db, ord, user_pk, perms).await?;
                    if ver.is_some() {
                        return Ok(ver);
                    }
                }
            }
        }
    } else {
        return Ok(Some(RequestError::SignatureError));
    }
    Ok(None)
}

/// Verify user orders
async fn verify_order(db: &Pool<Sqlite>, ord: &Order, user_pk: &PublicKey, perms: &Permissions) -> Result<Option<RequestError>, Box<dyn Error + Send + Sync>> {
    match ord {
        Order::AddUser { publickey, .. } | Order::AddMasterNode { publickey, .. } | Order::AddNode { publickey, .. } => {
            match publickey.try_to_vec() {
                Ok(pk) => {
                    if super::db_calls::entity_with_pubkey_exists(db, &pk).await? {
                        return Ok(Some(RequestError::DuplicateEntity));
                    }
                },
                Err(_) => return Ok(Some(RequestError::InvalidEntity))
            }
        }
        Order::EditUser { publickey, .. } | Order::RemoveUser { publickey, .. }
        | Order::EditMasterNode { publickey, .. } | Order::RemoveMasterNode { publickey, .. }
        | Order::EditNode { publickey, .. } | Order::RemoveNode { publickey, .. }
        | Order::UpdatePublicKey { publickey, .. } | Order::ChangeNodeConfig { publickey, .. } => {
            match publickey.try_to_vec() {
                Ok(pk) => {
                    if !super::db_calls::entity_with_pubkey_exists(db, &pk).await? {
                        return Ok(Some(RequestError::InvalidEntity));
                    }
                },
                Err(_) => return Ok(Some(RequestError::InvalidEntity))
            }
        },
        Order::UpdateVoteWeight { publickey, .. } => {
            match publickey.try_to_vec() {
                Ok(pk) => {
                    if !super::db_calls::master_with_pubkey_exists(db, &pk).await? {
                        return Ok(Some(RequestError::InvalidEntity));
                    }
                },
                Err(_) => return Ok(Some(RequestError::InvalidEntity))
            }
        },
        Order::SetConfig { name, .. } => {
            if name.len() > 30 {
                return Ok(Some(RequestError::LongParameter));
            }
        },
        Order::RemoveConfig { name, .. } => {
            if super::db_calls::config_used(db, &name).await? {
                return Ok(Some(RequestError::ConfigUsed));
            }
        },
    }

    match ord {
        Order::AddUser { name, perm, .. } => {
            if name.len() > 30 {
                return Ok(Some(RequestError::LongParameter));
            } else if !perms.0.contains(&Permission::Root) {
                return Ok(Some(RequestError::PermissionError));
            } else if perm.0.is_empty() {
                return Ok(Some(RequestError::NoPermission));
            }
        },
        Order::AddMasterNode { name, .. } => {
            if name.len() > 30 {
                return Ok(Some(RequestError::LongParameter));
            } else if !perms.0.contains(&Permission::Root) {
                return Ok(Some(RequestError::PermissionError));
            }
        },
        Order::AddNode { name, .. } => {
            if name.len() > 30 {
                return Ok(Some(RequestError::LongParameter));
            } else if !perms.0.contains(&Permission::Root) {
                return Ok(Some(RequestError::PermissionError));
            }
        },
        Order::EditUser { name, publickey, perm } => {
            if name.len() > 30 {
                return Ok(Some(RequestError::LongParameter));
            } else if user_pk.eq(publickey) && !perms.0.contains(&Permission::Root) {
                if !perm.0.iter().all(|item| perms.0.contains(item)) {
                    return Ok(Some(RequestError::PermissionError));
                }
            } else if !perms.0.contains(&Permission::Root) {
                return Ok(Some(RequestError::PermissionError));
            }
        },
        Order::EditMasterNode { name, .. } => {
            if name.len() > 30 {
                return Ok(Some(RequestError::LongParameter));
            } else if !perms.0.contains(&Permission::Root) {
                return Ok(Some(RequestError::PermissionError));
            }
        },
        Order::EditNode { name, .. } => {
            if name.len() > 30 {
                return Ok(Some(RequestError::LongParameter));
            } else if !perms.0.contains(&Permission::Root) {
                return Ok(Some(RequestError::PermissionError));
            }
        },
        Order::UpdatePublicKey { publickey, .. } => {
            if !user_pk.eq(publickey) || !perms.0.contains(&Permission::Root) {
                return Ok(Some(RequestError::PermissionError));
            }
        },
        Order::UpdateVoteWeight { .. }
        | Order::RemoveUser { .. } | Order::RemoveMasterNode { .. } | Order::RemoveNode { .. }
        | Order::SetConfig { .. } | Order::RemoveConfig { .. } => {
            if !perms.0.contains(&Permission::Root) {
                return Ok(Some(RequestError::PermissionError));
            }
        },
        Order::ChangeNodeConfig { name, .. } => {
            if !perms.0.contains(&Permission::Root) {
                return Ok(Some(RequestError::PermissionError));
            }
            if !super::db_calls::config_exists(db, &name).await? {
                return Ok(Some(RequestError::InvalidConfig));
            }
        }
    }

    Ok(None)
}
