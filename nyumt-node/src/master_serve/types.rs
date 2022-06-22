use crate::{
    utils::keys::PublicKey,
    master::types::Permissions
};
use tokio::sync::oneshot::Sender;

/// User info
#[derive(Debug)]
pub struct User {
    pub name: String,
    pub pubkey: PublicKey,
    pub permissions: Permissions,
    pub last_access: u64,
    pub disconnected: Sender<()>
}
