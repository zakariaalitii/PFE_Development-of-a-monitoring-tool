use tracing::{error, debug};
use systemstat::{System, Platform};
use std::io::Error;
use pnet::datalink;
use borsh::{BorshSerialize, BorshDeserialize};
use enum_iterator::IntoEnumIterator;
use strum::IntoStaticStr;

use super::{VarValue, DataType};

#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct NetConstDbVal {
    // Collection of every interface name and it's id
    pub iface_name_id: Vec<String>,
    pub iface_mac: Vec<String>,
    pub iface_ip: Vec<Vec<String>>,
    pub iface_up: Vec<bool>
}

impl NetConstDbVal {
    pub fn new() -> NetConstDbVal {
        NetConstDbVal {
            iface_name_id: vec![],
            iface_mac: vec![],
            iface_ip: vec![],
            iface_up: vec![],
        }
    }
}

#[repr(u8)]
#[derive(Debug, BorshDeserialize, BorshSerialize, serde::Serialize, serde::Deserialize, PartialEq, Clone, IntoEnumIterator, IntoStaticStr)]
pub enum NetOID {
    NAME = 0,
    MAC,
    IP_ADDRESS,
    STATUS,
    RX_PACKETS,
    TX_PACKETS,
    RX_BYTES,
    TX_BYTES,
    RX_ERRORS,
    TX_ERRORS
}

pub type NetworkUsage = Vec<Vec<(NetOID, VarValue)>>;

/*#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct NetworkUsage(pub Vec<Network>);

#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct Network {
    pub name: NetId,
    pub rx_packets: Option<u64>, // rx packets received
    pub tx_packets: Option<u64>, // tx packets transmitted
    pub rx_bytes: Option<u64>, // bytes received
    pub tx_bytes: Option<u64>, // bytes transmitted
    pub rx_errors: Option<u64>, // number of errors for received packets
    pub tx_errors: Option<u64>  // number of errors for transmitted packets
}*/

pub fn interpret_oid(oid: &Option<NetOID>) -> Vec<NetOID> {
    let mut vec = vec![];
    match oid {
        Some(v) => vec.push(v.clone()),
        None => {
            for _oid in NetOID::into_enum_iter() {
                vec.push(_oid);
            }
        }
    }
    vec
}

pub async fn usage(sys: &System, db_struct: &Vec<NetOID>) -> Result<DataType, Error> {
    match sys.networks() {
        Ok(netifs) => {
            let mut nets = Vec::new();
            for netif in netifs.values() {
                let mut net: Vec<(NetOID, VarValue)> = Vec::new();
                match sys.network_stats(&netif.name) {
                    Ok(v) => {
                        for netw in datalink::interfaces() {
                            if netw.name == netif.name {
                                let (iface_name, mac, status, ip) = (VarValue::Str(netif.name.clone()),
                                    VarValue::Str(match netw.mac {
                                        Some(v) => v.to_string(),
                                        None => "".to_owned()
                                    }),
                                    VarValue::Bool(netw.is_up()),
                                    {
                                        let mut ip_list = Vec::new();
                                        for ip_addr in netw.ips {
                                            ip_list.push(ip_addr.to_string());
                                        }
                                        VarValue::VecStr(ip_list)
                                    });

                                debug!("Network {}: {} rx packets, {} tx packets, {} rx bytes, {} tx bytes, {} rx errors, {} tx errors",
                                        netif.name, v.rx_packets, v.tx_packets, v.rx_bytes, v.tx_bytes, v.rx_errors, v.tx_errors
                                      );
                                for i in db_struct {
                                    match i {
                                        NetOID::NAME => net.push((NetOID::NAME, iface_name.clone())),
                                        NetOID::MAC => net.push((NetOID::MAC, mac.clone())),
                                        NetOID::IP_ADDRESS => net.push((NetOID::IP_ADDRESS, ip.clone())),
                                        NetOID::STATUS => net.push((NetOID::STATUS, status.clone())),
                                        NetOID::RX_PACKETS => net.push((NetOID::RX_PACKETS, VarValue::Number(v.rx_packets))),
                                        NetOID::TX_PACKETS => net.push((NetOID::TX_PACKETS, VarValue::Number(v.tx_packets))),
                                        NetOID::RX_BYTES => net.push((NetOID::RX_BYTES, VarValue::Number(v.rx_bytes.0))),
                                        NetOID::TX_BYTES => net.push((NetOID::TX_BYTES, VarValue::Number(v.tx_bytes.0))),
                                        NetOID::RX_ERRORS => net.push((NetOID::RX_ERRORS, VarValue::Number(v.rx_errors))),
                                        NetOID::TX_ERRORS => net.push((NetOID::TX_ERRORS, VarValue::Number(v.tx_errors))),
                                    }
                                }
                                if !net.is_empty() {
                                    nets.push(net);
                                }
                                break;
                            }
                        }
                    },
                    Err(e) => {
                        error!("Networks: error getting info for network {}: {}", netif.name, e);
                    }
                }
            }
            return Ok(DataType::Network(nets));
        }
        Err(e) => {
            error!("Networks: error: {}", e);
            return Err(e);
        },
    }
}
