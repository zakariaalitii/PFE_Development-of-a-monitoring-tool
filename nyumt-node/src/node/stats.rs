use nyumt_proto::{stats::{self, System, Platform}, peer::PeerRequest};
use std::sync::Arc;
use tokio::time::{self, Duration};
use borsh::BorshSerialize;
use tracing::{warn, error};
use libp2p::identity::Keypair;
use nyumt_proto::types::NodeReport;

use super::SwarmSettings;
use crate::utils::keys::PublicKey;
use super::activity::SwarmHandlerCall;

pub async fn collector(mut swarm_handle: SwarmHandlerCall, swarm_set: Arc<SwarmSettings>, local_node_keypair: Keypair, pk: PublicKey) {
    tokio::task::spawn(async move {
        let db            = swarm_set.db.clone();
        let sys           = Arc::new(System::new());
        let mut conf      = swarm_set.conf.read().await.clone();
        let mut db_struct = Arc::new(conf.db_struct);
        let mut interval  = time::interval(Duration::from_secs(conf.report_timeframe));
        let ser_pk        = match pk.try_to_vec() {
            Ok(v) => v,
            Err(e) => {
                warn!("Error parsing local node public key, stopping now: {}", e);
                swarm_set.stop.clone().send(crate::Stop::Panic).await.ok();
                return;
            }
        };
        loop {
            interval.tick().await;
            match stats::generate_report(&sys, &db_struct).await {
                Ok(report) => {
                    match report.try_to_vec() {
                        Ok(v)  => {
                            match local_node_keypair.sign(&v) {
                                Ok(sig) => {
                                    let rep = PeerRequest::Report(NodeReport { report: v, sig });
                                    match rep.try_to_vec() {
                                        Ok(ser_rep) => {
                                            // TODO: Store reports
                                            match swarm_handle.publish_report(ser_pk.clone(), report.timestamp, ser_rep).await {
                                                Ok(_) => (),
                                                Err(e) => error!("Error passing report to channel: {}", e)
                                            }
                                        },
                                        Err(e) => error!("Error serializing report: {}", e)
                                    }
                                },
                                Err(e)  => error!("Error signing record: {}", e)
                            }
                        },
                        Err(e) => error!("Error serializing report: {}", e)
                    }
                },
                Err(_) => ()
            };
            if swarm_set.conf.read().await.hash != conf.hash {
                // Something changed, we need to update our config
                conf      = swarm_set.conf.read().await.clone();
                db_struct = Arc::new(conf.db_struct);
                if conf.report_timeframe != interval.period().as_secs() {
                    interval = time::interval(Duration::from_secs(conf.report_timeframe));
                }
            }
        };
    }).await.ok();
}
