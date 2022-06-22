use borsh::BorshSerialize;
use zeroize::Zeroize;
use libp2p::{
    core::{InboundUpgrade, OutboundUpgrade, PeerId, UpgradeInfo},
    noise::{NoiseConfig, RemoteIdentity, NoiseOutput, NoiseError, Protocol}
};
use std::pin::Pin;
use futures::prelude::*;

use crate::database::{Database, node};
use crate::utils::keys::PublicKey;

#[derive(Clone)]
pub struct NoiseAuth<P, C: Zeroize, R> {
    pub config: NoiseConfig<P, C, R>,
    pub db: Database
}

impl<P, C: Zeroize, R> UpgradeInfo for NoiseAuth<P, C, R>
where
    NoiseConfig<P, C, R>: UpgradeInfo,
{
    type Info = <NoiseConfig<P, C, R> as UpgradeInfo>::Info;
    type InfoIter = <NoiseConfig<P, C, R> as UpgradeInfo>::InfoIter;

    fn protocol_info(&self) -> Self::InfoIter {
        self.config.protocol_info()
    }
}

impl<T, P, C, R> InboundUpgrade<T> for NoiseAuth<P, C, R>
where
    NoiseConfig<P, C, R>: UpgradeInfo
        + InboundUpgrade<T, Output = (RemoteIdentity<C>, NoiseOutput<T>), Error = NoiseError>
        + 'static,
    <NoiseConfig<P, C, R> as InboundUpgrade<T>>::Future: Send,
    T: AsyncRead + AsyncWrite + Send + 'static,
    C: Protocol<C> + AsRef<[u8]> + Zeroize + Send + 'static,
{
    type Output = (PeerId, NoiseOutput<T>);
    type Error = NoiseError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + Send>>;

    fn upgrade_inbound(self, socket: T, info: Self::Info) -> Self::Future {
        let db = self.db.clone();
        Box::pin(
            self.config
                .upgrade_inbound(socket, info)
                .and_then(move |(remote, io)| match remote {
                    RemoteIdentity::IdentityKey(pk) => {
                        match &pk {
                            libp2p::core::PublicKey::Ed25519(ed) => {
                                match PublicKey::ED25519(ed.encode()).try_to_vec() {
                                    Ok(v) => {
                                        match futures::executor::block_on(node::get_node_id(&db, &v)) {
                                            Ok(Some(_)) => future::ok((pk.to_peer_id(), io)),
                                            _ => future::err(NoiseError::AuthenticationFailed)
                                        }
                                    },
                                    Err(_) => future::err(NoiseError::AuthenticationFailed)
                                }
                            },
                            _ => future::err(NoiseError::AuthenticationFailed)
                        }
                    },
                    _ => future::err(NoiseError::AuthenticationFailed),
                }),
        )
    }
}

impl<T, P, C, R> OutboundUpgrade<T> for NoiseAuth<P, C, R>
where
    NoiseConfig<P, C, R>: UpgradeInfo
        + OutboundUpgrade<T, Output = (RemoteIdentity<C>, NoiseOutput<T>), Error = NoiseError>
        + 'static,
    <NoiseConfig<P, C, R> as OutboundUpgrade<T>>::Future: Send,
    T: AsyncRead + AsyncWrite + Send + 'static,
    C: Protocol<C> + AsRef<[u8]> + Zeroize + Send + 'static,
{
    type Output = (PeerId, NoiseOutput<T>);
    type Error = NoiseError;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + Send>>;

    fn upgrade_outbound(self, socket: T, info: Self::Info) -> Self::Future {
        let db = self.db.clone();
        Box::pin(
            self.config
                .upgrade_outbound(socket, info)
                .and_then(move |(remote, io)| match remote {
                    RemoteIdentity::IdentityKey(pk) => {
                        match &pk {
                            libp2p::core::PublicKey::Ed25519(ed) => {
                                match PublicKey::ED25519(ed.encode()).try_to_vec() {
                                    Ok(v) => {
                                        match futures::executor::block_on(node::get_node_id(&db, &v)) {
                                            Ok(Some(_)) => future::ok((pk.to_peer_id(), io)),
                                            _ => future::err(NoiseError::AuthenticationFailed)
                                        }
                                    },
                                    Err(_) => future::err(NoiseError::AuthenticationFailed)
                                }
                            },
                            _ => future::err(NoiseError::AuthenticationFailed)
                        }
                    },
                    _ => future::err(NoiseError::AuthenticationFailed),
                }),
        )
    }
}
