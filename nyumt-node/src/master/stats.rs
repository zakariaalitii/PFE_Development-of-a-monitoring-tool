use nyumt_proto::stats::{self, System, Platform};
use std::sync::Arc;
use tokio::{
    sync::RwLock,
    time::{self, Duration}
};
use tracing::{warn, error};
use libp2p::identity::Keypair;
use borsh::{BorshSerialize, BorshDeserialize};

use super::SwarmSettings;
use crate::database::node::{store_stats, self};
use crate::utils::keys::PublicKey;
use crate::database::cache;

pub async fn collector(swarm_set: Arc<SwarmSettings>, local_node_keypair: Keypair, pk: PublicKey) {
    tokio::task::spawn(async move {
        let db            = swarm_set.db.clone();
        let sys           = Arc::new(System::new());
        let mut conf      = swarm_set.conf.read().await.node.clone();
        let mut db_struct = Arc::new(conf.db_struct);
        let mut interval  = time::interval(Duration::from_secs(conf.report_timeframe));
        let ser_pk        = match pk.try_to_vec() {
            Ok(v) => v,
            Err(e) => {
                warn!("Error parsing local node publick key, stopping now: {}", e);
                swarm_set.stop.clone().send(crate::Stop::Panic).await.ok();
                return;
            }
        };
        let id       = match node::get_node_id(&db, &ser_pk).await {
            Ok(Some(v)) => v,
            _           => {
                warn!("Error fetching node id from database");
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
                                    match store_stats(&db, &ser_pk, &v, &sig, report.timestamp).await {
                                        Ok(_) => (),
                                        Err(e) => error!("Error storing report: {}", e)
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
            if swarm_set.conf.read().await.node.hash != conf.hash {
                // Something changed, we need to update our config
                conf      = swarm_set.conf.read().await.node.clone();
                db_struct = Arc::new(conf.db_struct);
                if conf.report_timeframe != interval.period().as_secs() {
                    interval = time::interval(Duration::from_secs(conf.report_timeframe));
                }
            }
        };
    }).await.ok();
}
