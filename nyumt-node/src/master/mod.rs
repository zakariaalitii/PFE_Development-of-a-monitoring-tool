mod peer;
mod activity;
pub mod types;
pub mod net_behavior;
mod stats;
pub mod first;
pub mod req_resp;
mod noise_transport;

use borsh::{BorshSerialize, BorshDeserialize};
use nyumt_proto::{keys::PublicKey, config::{NodeConfig, MasterConfig}, peer::PeerResponse};
use tokio::sync::{mpsc, RwLock, oneshot};
use std::{
    error::Error, iter,
    time::Duration,
    sync::Arc, collections::HashMap
};
use libp2p::{
    Multiaddr,
    gossipsub, identity,
    swarm::SwarmBuilder,
    PeerId,
    Transport, noise, tcp::TokioTcpConfig, mplex, core::upgrade,
    mdns::Mdns,
    request_response::{RequestResponse, ProtocolSupport, RequestId},
    gossipsub::{
        GossipsubMessage, IdentTopic as Topic, MessageAuthenticity, ValidationMode,
        MessageId
    },
    ping
};
use tracing::error;
use sha2::{Sha256, Digest};

use crate::{Settings, DaemonSettings, database::cache};
use crate::utils::keys::LocalPeerKey;
use crate::database::block;
use crate::config::GlobalConfig;

pub struct SwarmSettings {
    /// Handle to the swarm
    swarm: activity::SwarmHandlerCall,
    /// Vote Weight for local node
    pub vote: u32,
    /// Channel to submit new blocks
    pub channel: mpsc::Sender<types::NetEvent>,
    /// Database connection
    pub db: crate::database::Database,
    /// Configuration
    pub conf: Arc<RwLock<GlobalConfig>>,
    /// Current configuration
    pub conf_name: Arc<RwLock<String>>,
    /// Pending block with their vote count (block, height, vote weight by case, number of voters)
    pub pending_blocks: Arc<RwLock<Vec<([u8; 32], TotalVote, u32)>>>,
    /// Approved blocks waiting for their id and hash
    pub approved_blocks: Arc<RwLock<Vec<(u64, [u8; 32], TotalVote, u32)>>>,
    /// Topic
    pub topic: Topic,
    /// A channel to send stop requests to runtime
    pub stop: mpsc::Sender<crate::Stop>,
    /// Hash of subscribed nodes with their current config
    pub subscribed_nodes: Arc<RwLock<HashMap<String, Vec<PeerId>>>>,
    /// Routable address
    pub addr: Arc<RwLock<Option<Multiaddr>>>,
    /// Hold responses
    pub resp: Arc<RwLock<HashMap<RequestId, oneshot::Sender<PeerResponse>>>>,
    /// Timeout
    pub timeout: Duration,
    /// Keypair
    pub keypair: libp2p::identity::Keypair,
    /// Public key of local node
    pub pubkey: PublicKey
}

/// Total votes
pub struct TotalVote {
    ok: u32,
    no: u32,
}

pub async fn serve(settings: &mut Settings, daemon_conf: &DaemonSettings,
                   recv: &mut mpsc::Receiver<types::Request>, send: mpsc::Sender<types::Request>,
                   db: crate::database::Database,
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

    let topic = Topic::new("nyumt-master");

    let (tx, rx) = mpsc::channel(200);

    let (swarm_tx, swarm_rx) = mpsc::channel(100);
    let mut swarm_handler    = activity::SwarmHandlerCall { tx: swarm_tx };

    let (glob, conf) = match cache::load_local_master_node_conf(&db, &keys.public().try_to_vec()?).await {
        Ok(v) => match v {
            Some(v) => {
                let master = MasterConfig::try_from_slice(&v.1);
                let node   = NodeConfig::try_from_slice(&v.2);
                if master.is_err() || node.is_err() {
                    error!("Error serializing config from cache, stopping now");
                    stop.send(crate::Stop::Panic).await.ok();
                    return Ok(());
                }
                let config = GlobalConfig { master: master.unwrap(), node: node.unwrap() };
                (v.0, config)
            },
            None => {
                error!("Config for node should be defined at this point");
                stop.send(crate::Stop::Panic).await.ok();
                return Ok(());
            }
        },
        Err(e) => {
            error!("Error loading config from cache: {}", e);
            stop.send(crate::Stop::Panic).await.ok();
            return Ok(());
        }
    };

    let swarm_set   = Arc::new(SwarmSettings {
        swarm: swarm_handler.clone(),
        db: db.clone(),
        vote: 1,
        conf: Arc::new(RwLock::new(conf)),
        conf_name: Arc::new(RwLock::new(glob)),
        pending_blocks: Arc::new(RwLock::new(Vec::new())),
        approved_blocks: Arc::new(RwLock::new(Vec::new())),
        channel: tx.clone(),
        topic: topic.clone(),
        stop,
        subscribed_nodes: Arc::new(RwLock::new(HashMap::new())),
        addr: Arc::new(RwLock::new(None)),
        resp: Arc::new(RwLock::new(HashMap::new())),
        timeout: Duration::from_secs(settings.timeout as u64),
        keypair: local_key.clone(),
        pubkey: keys.public()
    });


    // Set up a an encrypted DNS-enabled TCP Transport over the Mplex protocol.
    let transport = TokioTcpConfig::new()
        .nodelay(true)
        .upgrade(upgrade::Version::V1)
        .authenticate(noise_transport::NoiseAuth { config: noise::NoiseConfig::xx(noise_keys), db })
        .multiplex(mplex::MplexConfig::new())
        .boxed();

    // Create a Swarm to manage peers and events
    let mut swarm = {
        // To content-address message, we can take the hash of message and use it as an ID.
        let message_id_fn = |message: &GossipsubMessage| {
            let mut hasher = Sha256::new();
            hasher.update(message.data.as_slice());
            MessageId::from(format!("{:X}", hasher.finalize()))
        };

        // Set a custom gossipsub
        let gossipsub_config = gossipsub::GossipsubConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(10)) // This is set to aid debugging by not cluttering the log space
            .validation_mode(ValidationMode::Strict) // This sets the kind of message validation. The default is Strict (enforce message signing)
            .message_id_fn(message_id_fn) // content-address messages. No two messages of the
            .max_transmit_size(4096)
            // same content will be propagated.
            .build()
            .expect("Valid config");
        // build a gossipsub network behaviour
        let gossipsub: gossipsub::Gossipsub =
            gossipsub::Gossipsub::new(MessageAuthenticity::Signed(local_key.clone()), gossipsub_config)
                .expect("Correct configuration");

        let mdns = Mdns::new(Default::default()).await?;

        let ping = ping::Behaviour::new(ping::Config::new().with_keep_alive(true));

        let net_behavior = net_behavior::NyumtProt {
            gossipsub,
            mdns,
            sett: swarm_set.clone(),
            req_resp: RequestResponse::new(
                net_behavior::NyumtReqRespCodec(),
                iter::once((net_behavior::NyumtReqRespProt(), ProtocolSupport::Full)),
                Default::default(),
            ),
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

    // Connect to all master nodes
    if !settings.peers.is_empty() {
        let join = settings.peers.pop().unwrap();
        let addr: libp2p::Multiaddr = join.addr.parse()?;
        match swarm.dial(addr) {
            Ok(_)  => {
                let mut settings_file = crate::NYUMT_DIR.clone();
                settings_file.push("node_settings.json");
                tokio::fs::write(settings_file.to_str().unwrap(), serde_json::to_string(&settings)?.as_bytes()).await?;
            },
            Err(e) => {
                error!("Error joining network, master node must be active in the first time: {}", e);
                swarm.behaviour_mut().sett.stop.clone().send(crate::Stop::Panic).await.ok();
                return Ok(());
            },
        }
        swarm.behaviour_mut().gossipsub.add_explicit_peer(&PeerId::from_public_key(&PublicKey::parse_str(&join.publickey).unwrap().to_publickey().unwrap()));
    }

    // Listen on all interfaces and whatever port the OS assigns
    swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{}", settings.local_peer_port).parse()?)?;

    // Spawning event handler for spawn
    tokio::spawn(async move {
        activity::swarm_event_handler(swarm, swarm_rx).await;
    });

    // Start our node monitoring report loop
    //stats::collector(swarm_set.clone(), local_key, keys.public()).await;
    
    // Check if chain is configured
    if block::get_last_block(&swarm_set.db.clone()).await?.is_none() {
        //activity::new_chain(&mut swarm, &swarm_set, recv).await?;
    } else if !peer::multi_master(&swarm_set.db.clone()).await {
        // If there are some peers we need to sync with them first
    }

    // Start receiving new incoming orders
    swarm_handler.subscribe(topic).await;
    activity::sync(&swarm_set, recv, rx, tx, swarm_handler, keys.public()).await?;

    Ok(())
}
