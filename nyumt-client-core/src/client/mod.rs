pub mod activity;
pub mod behavior;
pub mod req_resp;

use std::collections::HashMap;
use std::{
    error::Error, sync::Arc,
    iter
};
use borsh::BorshSerialize;
use libp2p::request_response::RequestId;
use nyumt_proto::api::ClientRequest;
use nyumt_proto::stats::OID;
use tokio::sync::{RwLock, mpsc};
use tokio::sync::oneshot;
use tracing::error;
use libp2p::{
    swarm::SwarmBuilder,
    PeerId,
    Transport, noise, tcp::TokioTcpConfig, mplex, core::upgrade,
    request_response::{RequestResponse, ProtocolSupport},
    ping, Multiaddr
};

use nyumt_proto::{
    api::{Request, RequestResp},
    keys::PublicKey,
    database::{block, node}
};

use crate::{db::Database, storage::Storage};
use crate::POLL_TIME;

pub struct Client {
    swarm_handle: activity::SwarmHandlerCall,
    db: Database,
    host: String,
    storage: Arc<RwLock<Storage>>,
    session_id: usize,
    peer: PeerId
}

pub struct SwarmSettings {
    /// Database connection
    pub db: Database,
    /// Public key
    pub publickey: PublicKey,
    /// List of connected client
    pub swarm_handle: activity::SwarmHandlerCall,
    /// Signal when we get disconnected
    pub disconnected: Arc<RwLock<Option<oneshot::Sender<()>>>>,
    /// Signal when we want to close connection
    pub close_connection: Arc<RwLock<Option<oneshot::Receiver<()>>>>,
    /// A response to pending request
    pub resp: Arc<RwLock<HashMap<RequestId, oneshot::Sender<RequestResp>>>>,
    /// Peerid of master node we are connected to
    pub peer: Arc<RwLock<Option<PeerId>>>,
    /// Last block gotten
    pub last_height: Arc<RwLock<i64>>
}

impl Client {
    pub async fn new(storage: Arc<RwLock<Storage>>, session_id: usize, db: Database) -> Result<(Client, oneshot::Receiver<()>, oneshot::Sender<()>), Box<dyn Error>> {

        let local_key;
        let publickey;
        let host;
        {
            let soft_lock = storage.read().await;
            let ses       = &soft_lock.get().session.0[session_id];
            publickey     = ses.keys.public();
            local_key     = ses.keys.to_keypair()?;
            host          = ses.host.0.to_owned();
        }
        
        let local_peer_id = PeerId::from(local_key.public());

        let noise_keys = match noise::Keypair::<noise::X25519Spec>::new()
            .into_authentic(&local_key) {
            Ok(v)  => v,
            Err(e) => {
                error!("Signing libp2p-noise static DH keypair failed.");
                return Err( Box::new( std::io::Error::new( std::io::ErrorKind::Other, e ) ) )
            }
        };

        let (swarm_tx, swarm_rx) = mpsc::channel(100);

        let mut swarm_handle = activity::SwarmHandlerCall { tx: swarm_tx };

        let (tx, rx)   = oneshot::channel();
        let (tx1, rx1) = oneshot::channel();

        let peer = Arc::new(RwLock::new(None));

        let last_height;
        let last_timestamp;
        {
            let height = match block::get_last_block_height(&db).await {
                Ok(v) => v,
                Err(e) => {
                    error!("Error getting height from database: {}", e);
                    return Err( Box::new( std::io::Error::new( std::io::ErrorKind::Other, e ) ) )
                }
            };
            last_height = if height.is_some() { height.unwrap() as i64 } else { -1 };
        

            let height = match node::get_last_stats_timestamp(&db).await {
                Ok(v) => v,
                Err(e) => {
                    error!("Error getting height from database: {}", e);
                    return Err( Box::new( std::io::Error::new( std::io::ErrorKind::Other, e ) ) )
                }
            };
            last_timestamp = if height.is_some() { height.unwrap() } else { 0 };
        }
        let swarm_set = Arc::new(SwarmSettings {
            db: db.clone(),
            publickey: publickey.clone(),
            swarm_handle: swarm_handle.clone(),
            disconnected: Arc::new(RwLock::new(Some(tx))),
            close_connection: Arc::new(RwLock::new(Some(rx1))),
            resp: Arc::new(RwLock::new(HashMap::new())),
            peer: peer.clone(),
            last_height: Arc::new(RwLock::new(last_height))
        });

        // Create a Swarm to manage peers and events
        let mut swarm = {
            let ping = ping::Behaviour::new(ping::Config::new().with_keep_alive(true));

            let transport = TokioTcpConfig::new()
                .nodelay(true)
                .upgrade(upgrade::Version::V1)
                .authenticate(noise::NoiseConfig::xx(noise_keys).into_authenticated())
                .multiplex(mplex::MplexConfig::new())
                .boxed();

            let net_behavior = behavior::NyumtClientProt {
                req_resp: RequestResponse::new(
                    behavior::NyumtReqRespCodec(),
                    iter::once((behavior::NyumtReqRespProt(), ProtocolSupport::Full)),
                    Default::default(),
                ),
                sett: swarm_set,
                ping
            };

            // build the swarm
            SwarmBuilder::new(transport, net_behavior, local_peer_id)
                // We want the connection background tasks to be spawned
                // onto the tokio runtime.
                .executor(Box::new(|fut| {
                    tokio::spawn(fut);
            }))
            .build()
        };

        let ip_port = host.split(":").collect::<Vec<&str>>();
        if ip_port.len() != 2 {
            return Err(Box::new( std::io::Error::new( std::io::ErrorKind::Other, format!("Error parsing [ip|domain]:port from {}", host) ) ));
        }
        let m: Multiaddr = format!("/ip4/{}/tcp/{}", ip_port[0], ip_port[1]).parse()?;
        match swarm.dial(m.clone()) {
            Ok(_) => (),
            Err(e) => return Err(Box::new(e))
        }

        swarm.listen_on(format!("/ip4/0.0.0.0/tcp/0").parse()?)?;

        // Handling events coming
        tokio::spawn(async move {
            activity::handler(swarm, swarm_rx).await.ok();
        });

        loop {
            if peer.read().await.is_some() {
                break;
            }
            tokio::time::sleep(*POLL_TIME).await;
        }
        
        let peeropt = peer.write().await.unwrap();
        swarm_handle.hello(&peeropt, &publickey, last_height, last_timestamp).await.ok();
        
        Ok((Client {
            swarm_handle,
            db,
            host: m.to_string(),
            storage,
            session_id,
            peer: peeropt
        }, rx, tx1))
    }

    pub async fn execute(&mut self, req: Request) -> Result<RequestResp, Box<dyn Error>> {
        let req  = ClientRequest::Request(req).try_to_vec()?;
        let rx   = self.swarm_handle.send_order(&self.peer, req).await.unwrap();
        let resp = rx.await?;

        Ok(resp)
    }

    pub async fn get_stats(&mut self, pk: PublicKey, oid: OID) -> Result<RequestResp, Box<dyn Error>> {
        let req  = ClientRequest::GetStats(pk, oid).try_to_vec()?;
        let rx   = self.swarm_handle.send_order(&self.peer, req).await.unwrap();
        let resp = rx.await?;

        Ok(resp)
    }
}
