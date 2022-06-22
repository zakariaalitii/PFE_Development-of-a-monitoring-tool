use clap::ArgMatches;
use super::Settings;
use pnet::datalink;
use libp2p::Multiaddr;
use std::error::Error;
use serde_json;
use tracing::error;

use super::JoinPeer;
use crate::utils::keys::PublicKey;

pub async fn handle(settings: &Settings, args: &ArgMatches, pk: PublicKey) {
    let expk = pk.export();
    println!("Node publickey: {}", expk);
    if args.is_present("join_address") {
        if !settings.master_node {
            println!("Skipping join addresses (Node is not a master node)");
        } else {
            println!("Join address(es):");
            for iface in datalink::interfaces() {
                if iface.is_up() && !iface.ips.is_empty() {
                    println!("\t{}:", iface.name);
                    let ipv4 = iface.ips[0].ip().is_ipv4();
                    let ip   = iface.ips[0].ip().to_string();
                    println!("\t\t{}\t: {}", ip, create_address(&ip, ipv4, settings.local_peer_port, &expk).await);
                }
            }
        }
    }
    if args.is_present("config") {
        println!("Config:");
        println!("\tLocal peer port\t\t\t: {}", settings.local_peer_port);
        println!("\tClient port\t\t\t: {}", settings.client_port);
        println!("\tGeneral Connection timeout\t: {}", settings.timeout);
        println!("\tNode type\t\t\t: {}", if settings.master_node { "Master" } else { "Normal" });
    }
}

pub async fn create_address(addr: &str, is_ipv4: bool, port: u16, pk: &str) -> String {
    let maddr: Multiaddr = format!("/{}/{}/tcp/{}", if is_ipv4 { "ip4" } else { "ip6" }, addr, port).parse().expect("Can't parse address");
    base64::encode(serde_json::to_string(&JoinPeer { addr: maddr.to_string(), publickey: pk.to_string() }).expect("Can't create JoinPeer address for display"))
}

pub async fn parse_peer(peer: &str) -> Result<JoinPeer, Box<dyn Error>> {
    let peer: JoinPeer = serde_json::from_slice(&base64::decode(peer)?)?;
    match PublicKey::parse_str(&peer.publickey) {
        Ok(_) => (),
        Err(e) => {
            error!("Error parsing publickey: {}", e);
            return Err(e);
        }
    }
    let _addr: Multiaddr = peer.addr.parse()?;
    Ok(peer)
}
