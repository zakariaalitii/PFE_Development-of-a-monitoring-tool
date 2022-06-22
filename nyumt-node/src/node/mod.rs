mod noise_transport;
pub mod net_behavior;
mod stats;
pub mod activity;
pub mod req_resp;

use std::{
    sync::Arc,
    error::Error,
    iter, collections::HashMap
};
use borsh::{BorshSerialize, BorshDeserialize};
use futures::StreamExt;
use nyumt_proto::{peer::PeerResponse, database::cache};
use tokio::sync::{mpsc, RwLock, oneshot};
use tracing::{error, info};
use libp2p::{
    identity,
    swarm::SwarmBuilder,
    PeerId,
    Transport, noise, tcp::TokioTcpConfig, mplex, core::upgrade,
    mdns::Mdns,
    request_response::{RequestResponse, ProtocolSupport, RequestId},
    ping, Multiaddr
};

use crate::{Settings, config::NodeConfig};
use crate::utils::keys::{LocalPeerKey, PublicKey};
use crate::database::node;

pub struct SwarmSettings {
    /// Database connection
    pub db: crate::database::Database,
    /// Node config
    pub conf: Arc<RwLock<NodeConfig>>,
    /// A channel to send stop requests to runtime
    pub stop: mpsc::Sender<crate::Stop>,
    /// Currently default master node
    pub default_master_node: Arc<RwLock<Option<PeerId>>>,
    /// Hold responses
    pub resp: Arc<RwLock<HashMap<RequestId, oneshot::Sender<PeerResponse>>>>,
    pub swarm: activity::SwarmHandlerCall,
    /// Keypair
    pub keypair: libp2p::identity::Keypair,
    /// Node public key
    pub pubkey: PublicKey
}

pub async fn serve(settings: &mut Settings, db: crate::database::Database,
                   keys: &mut LocalPeerKey, stop: mpsc::Sender<crate::Stop>) -> Result<(), Box<dyn Error + Sync + Send>> {
    // Create a random key for ourselves.
    let local_key = match keys {
        LocalPeerKey::ED25519(k) => identity::Keypair::Ed25519(k.clone())
    };
    let local_peer_id = PeerId::from(local_key.public());

    let noise_keys = match noise::Keypair::<noise::X25519Spec>::new()
        .into_authentic(&local_key) {
        Ok(v)  => v,
        Err(e) => {
            error!("Signing libp2p-noise static DH keypair failed.");
            return Err( Box::new( std::io::Error::new( std::io::ErrorKind::Other, e ) ) )
        }
    };

    let (swarm_tx, swarm_rx): (mpsc::Sender<activity::SwarmHandlerReq>, mpsc::Receiver<activity::SwarmHandlerReq>) = mpsc::channel(100);
    let mut swarm_handle = activity::SwarmHandlerCall { tx: swarm_tx };

    let conf = match cache::load_local_node_conf(&db).await {
        Ok(Some(v)) => match NodeConfig::try_from_slice(&v) {
            Ok(v) => v,
            _ => NodeConfig::default()
        },
        _ => NodeConfig::default()
    };

    let swarm_set = Arc::new(SwarmSettings {
        db,
        conf: Arc::new(RwLock::new(conf)),
        stop,
        default_master_node: Arc::new(RwLock::new(None)),
        resp: Arc::new(RwLock::new(HashMap::new())),
        pubkey: keys.public(),
        swarm: swarm_handle.clone(),
        keypair: local_key.clone()
    });

    // Create a Swarm to manage peers and events
    let mut swarm = {
        let mdns = Mdns::new(Default::default()).await?;

        let ping = ping::Behaviour::new(ping::Config::new().with_keep_alive(true));

        let transport = TokioTcpConfig::new().nodelay(true)
            .upgrade(upgrade::Version::V1)
            .authenticate(noise::NoiseConfig::xx(noise_keys).into_authenticated())
            .multiplex(mplex::MplexConfig::new())
            .boxed();

        let net_behavior = net_behavior::NyumtNodeProt {
            mdns,
            req_resp: RequestResponse::new(
                net_behavior::NyumtReqRespCodec(),
                iter::once((net_behavior::NyumtReqRespProt(), ProtocolSupport::Full)),
                Default::default(),
            ),
            sett: swarm_set.clone(),
            ping,
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

    // Connect to all master nodes
    if !settings.peers.is_empty() {
        let join = settings.peers.pop().unwrap();
        let addr: Multiaddr = join.addr.parse()?;
        match PublicKey::parse_str(&join.publickey) {
            Ok(v) => {
                let master = vec![(v.try_to_vec().unwrap(), Some(addr.to_string()))];
                futures::executor::block_on(node::add_master_nodes(&swarm_set.db.clone(), &master)).unwrap();
            },
            Err(e) => {
                error!("Got unparsable publickey in join address: {}", e);
                futures::executor::block_on(swarm_set.stop.clone().send(crate::Stop::Panic)).ok();
                return Ok(());
            }
        };

        match swarm.dial(addr) {
            Ok(_)  => {
                let mut settings_file = crate::NYUMT_DIR.clone();
                settings_file.push("node_settings.json");
                tokio::fs::write(settings_file.to_str().unwrap(), serde_json::to_string(&settings)?.as_bytes()).await?;
            },
            Err(e) => {
                error!("Error joining network, master node must be active in the first time: {}", e);
                swarm_set.stop.clone().send(crate::Stop::Panic).await.ok();
            },
        }
        info!("Connecting to network with join address: addr:{}, public_key:{}", join.addr, join.publickey);
    } else {
        let db        = swarm.behaviour().sett.db.clone();
        let mut nodes = node::node_get_master_nodes_addr(&db).await;
        while let Some(node) = nodes.next().await {
            match node {
                Ok(v) => {
                    match v.addr {
                        Some(v) => {
                            let addr: Multiaddr = v.parse()?;
                            swarm.dial(addr).ok();
                            break;
                        },
                        None => ()
                    }
                },
                Err(e) => {
                    error!("Error joining network: {}", e);
                    swarm_set.stop.clone().send(crate::Stop::Panic).await.ok();
                }
            }
        }
    }

    // Listen on specified interface
    swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{}", settings.local_peer_port).parse()?)?;

    // Handling events coming
    activity::handler(swarm, swarm_rx).await?;

    // Getting current nodes and config
    swarm_handle.synchronise().await.ok();

    // Start our node monitoring report loop
    stats::collector(swarm_handle, swarm_set.clone(), local_key, keys.public()).await;

    Ok(())
}
