mod types;
mod noise_transport;
pub mod behavior;
mod activity;
mod req_resp;

use std::{
    sync::Arc,
    error::Error,
    iter, collections::HashMap
};

use tokio::sync::{mpsc::{self, Sender}, RwLock};
use tracing::error;
use libp2p::{
    identity,
    swarm::SwarmBuilder,
    PeerId,
    Transport, noise, tcp::TokioTcpConfig, mplex, core::upgrade,
    request_response::{RequestResponse, ProtocolSupport},
    ping,
};

use crate::Settings;
use crate::utils::keys::LocalPeerKey;
use crate::master::types::Request;

pub struct SwarmSettings {
    /// Database connection
    pub db: crate::database::Database,
    /// List of connected client
    pub auth: Arc<RwLock<HashMap<PeerId, types::User>>>,
    pub swarm_handle: activity::SwarmHandlerCall,
    pub req_sender: Sender<Request>
}

pub async fn serve(settings: &mut Settings, db: crate::database::Database,
                   keys: &mut LocalPeerKey, stop: mpsc::Sender<crate::Stop>,
                   tx: Sender<Request>) -> Result<(), Box<dyn Error + Sync + Send>> {
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

    // Set up a an encrypted DNS-enabled TCP Transport over the Mplex protocol.
    let transport = TokioTcpConfig::new()
        .nodelay(true)
        .upgrade(upgrade::Version::V1)
        .authenticate(noise_transport::NoiseAuth { config: noise::NoiseConfig::xx(noise_keys), db: db.clone() })
        .multiplex(mplex::MplexConfig::new())
        .boxed();

    let (swarm_tx, swarm_rx) = mpsc::channel(100);

    let swarm_handle = activity::SwarmHandlerCall { tx: swarm_tx };

    let swarm_set = Arc::new(SwarmSettings {
        db,
        auth: Arc::new(RwLock::new(HashMap::new())),
        swarm_handle,
        req_sender: tx
    });

    // Create a Swarm to manage peers and events
    let mut swarm = {
        let ping = ping::Behaviour::new(ping::Config::new().with_keep_alive(true));

        let net_behavior = behavior::NyumtServerProt {
            req_resp: RequestResponse::new(
                behavior::NyumtReqRespCodec(),
                iter::once((behavior::NyumtReqRespProt(), ProtocolSupport::Full)),
                Default::default(),
            ),
            sett: swarm_set,
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

    swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{}", settings.client_port).parse()?)?;

    // Handling events coming
    activity::handler(swarm, swarm_rx).await?;
    
    Ok(())
}
