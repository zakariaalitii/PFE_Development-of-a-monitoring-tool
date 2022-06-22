use borsh::{BorshSerialize, BorshDeserialize};
use lazy_static::lazy_static;
use std::error::Error;
use async_stream::stream;
use futures::{ Stream, TryStreamExt };
use nyumt_proto::{keys::PublicKey, stats::{self, VarType, VarValueType, Report, VarValue, DataType, enum_iterator::IntoEnumIterator, OID, network::NetOID, storage::StorageOID}, database::{Database, node}};
use chrono::prelude::*;

lazy_static! {
    pub static ref DB_STRUCT: Vec<(&'static str, Vec<(&'static str, stats::VarType, stats::VarValueType)>)> = vec![
        ("CPU", vec![
            ("TOTAL", VarType::Volatile, VarValueType::Number),
            ("USER", VarType::Volatile, VarValueType::Number),
            ("NICE", VarType::Volatile, VarValueType::Number),
            ("SYSTEM", VarType::Volatile, VarValueType::Number),
            ("INT", VarType::Volatile, VarValueType::Number),
            ("IDLE", VarType::Volatile, VarValueType::Number)
        ]),
        ("MEMORY", vec![
            ("USED", VarType::Volatile, VarValueType::Number),
            ("TOTAL", VarType::Volatile, VarValueType::Number),
        ]),
        ("STORAGE", vec![
            ("NAME", VarType::Mandatory, VarValueType::Str),
            ("FILESYSTEM_TYPE", VarType::Stable, VarValueType::Str),
            ("FREE", VarType::Volatile, VarValueType::Number),
            ("TOTAL", VarType::Volatile, VarValueType::Number),
            ("TOTAL_NODES", VarType::Volatile, VarValueType::Number),
            ("FREE_NODES", VarType::Volatile, VarValueType::Number),
        ]),
        ("NETWORK", vec![
            ("NAME", VarType::Mandatory, VarValueType::Str),
            ("MAC", VarType::Stable, VarValueType::Str),
            ("IP_ADDRESS", VarType::Stable, VarValueType::Str),
            ("STATUS", VarType::Stable, VarValueType::VecStr),
            ("RX_PACKETS", VarType::Volatile, VarValueType::Number),
            ("TX_PACKETS", VarType::Volatile, VarValueType::Number),
            ("RX_BYTES", VarType::Volatile, VarValueType::Number),
            ("TX_BYTES", VarType::Volatile, VarValueType::Number),
            ("RX_ERRORS", VarType::Volatile, VarValueType::Number),
            ("TX_ERRORS", VarType::Volatile, VarValueType::Number)
        ])
    ];
}

pub async fn plot_data<'a>(db: &'a Database, pk: &PublicKey, oid: OID, timestamp: u64) -> Result<impl Stream<Item = Result<(String, String, Option<String>), Box<dyn Error + Send + Sync>>> + 'a + Unpin, Box<dyn Error>> {
    match oid {
        OID::Cpu(Some(_)) | OID::Memory(Some(_)) | OID::Network(Some(_)) | OID::Storage(Some(_)) => (),
        _ => return Err(Box::new( std::io::Error::new( std::io::ErrorKind::Other, format!("OID must be for a unique object") ) ))
    }

    let pk_buf    = pk.try_to_vec()?;
    let timestamp = if timestamp == 0 { match node::get_min_stats_timestamp_in_limit(&db, &pk_buf, 50).await.unwrap() { Some(v) => v, None => 0 } } else { timestamp };
    let mut data  = node::get_node_stats_from(&db, pk_buf, timestamp).await;

    Ok(Box::pin(stream! {
        loop {
            let next = data.try_next().await;
            match next? {
                Some(data) => {
                    let dt: DateTime<Utc> = DateTime::from_utc(NaiveDateTime::from_timestamp(data.1 as i64, 0), Utc);
                    let res = dt.format("%Y-%m-%d %H:%M:%S").to_string();
                    match Report::try_from_slice(&data.0) {
                        Ok(v) => {
                            for i in v.data {
                                match i {
                                    DataType::Cpu(v) => {
                                        match &oid {
                                            OID::Cpu(obj) => {
                                                let obj = obj.clone().unwrap();
                                                for val in v {
                                                    if val.0 == obj {
                                                        yield Ok((res.clone(), val.1.to_string(), None));
                                                        break;
                                                    }
                                                }
                                            },
                                            _ => ()
                                        }
                                    },
                                    DataType::Memory(v) => {
                                        match &oid {
                                            OID::Memory(obj) => {
                                                let obj = obj.clone().unwrap();
                                                for val in v {
                                                    if val.0 == obj {
                                                        yield Ok((res.clone(), val.1.to_string(), None));
                                                        break;
                                                    }
                                                }
                                            },
                                            _ => ()
                                        }
                                    },
                                    DataType::Network(v) => {
                                        match &oid {
                                            OID::Network(obj) => {
                                                let obj = obj.clone().unwrap();
                                                for val in v {
                                                    let mut name = None;
                                                    let mut var  = None;
                                                    for vv in val {
                                                        if vv.0 == obj {
                                                            var = Some(vv.1.to_string());
                                                        } else if vv.0 == NetOID::NAME {
                                                            name = Some(vv.1.to_string());
                                                        } else if var.is_some() {
                                                            yield Ok((res.clone(), var.unwrap(), name));
                                                            break;
                                                        }
                                                    }
                                                }
                                            },
                                            _ => ()
                                        }
                                    },
                                    DataType::Storage(v) => {
                                        match &oid {
                                            OID::Storage(obj) => {
                                                let obj = obj.clone().unwrap();
                                                for val in v {
                                                    let mut name = None;
                                                    let mut var  = None;
                                                    for vv in val {
                                                        if vv.0 == obj {
                                                            var = Some(vv.1.to_string());
                                                        } else if vv.0 == StorageOID::NAME {
                                                            name = Some(vv.1.to_string());
                                                        } else if var.is_some() {
                                                            yield Ok((res.clone(), var.unwrap(), name));
                                                            break;
                                                        }
                                                    }
                                                }
                                            },
                                            _ => ()
                                        }
                                    }
                                }
                            }
                        },
                        Err(_) => ()
                    }
                },
                None => break
            }
        }
    }))
} 

pub async fn interpret_stats(report: &Report) -> Vec<(String, VarValue)> {
    let mut vec = Vec::new();
    for rep in &report.data {
        match rep {
            DataType::Cpu(v) => format_oid_value(&mut vec, "0.CPU.{}", &v),
            DataType::Memory(v) => format_oid_value(&mut vec, "0.MEMORY.{}", &v),
            DataType::Network(v) => {
                let mut count = 0;
                for net in v {
                    for data in net {
                        let val: &'static str = data.0.to_owned().into();
                        vec.push((format!("0.NETWORK.{}[{}]", val, count), data.1.clone()));
                    }
                    count += 1;
                }
            },
            DataType::Storage(v) => {
                let mut count = 0;
                for storage in v {
                    for data in storage {
                        let val: &'static str = data.0.to_owned().into();
                        vec.push((format!("0.STORAGE.{}[{}]", val, count), data.1.clone()));
                    }
                    count += 1;
                }
            },
        }
    }

    vec
}

fn format_oid_value<T: Into<&'static str> + Clone>(vec: &mut Vec<(String, VarValue)>, format: &str, data_array: &Vec<(T, VarValue)>) {
    for data in data_array {
        let val: &'static str = data.0.to_owned().into();
        vec.push((format.replace("{}", val), data.1.clone()));
    }
}

pub async fn parse_oid(key: &str) -> Result<stats::OID, Box<dyn Error>> {
    let keys = key.split(".").collect::<Vec<&str>>();
    if keys.len() < 2 && keys.len() > 3 {
        return Err(Box::new( std::io::Error::new( std::io::ErrorKind::Other, format!("Error parsing OID {}", key) ) ));
    }

    if keys[0] != "0" {
        return Err(Box::new( std::io::Error::new( std::io::ErrorKind::Other, format!("Unknewn object database {}", keys[0]) ) ));
    }

    if keys[1] == "*" {
        if keys.len() == 2 || keys[2] == "*" {
            return Ok(stats::OID::All);
        } else {
            return Err(Box::new( std::io::Error::new( std::io::ErrorKind::Other, format!("Error parsing OID {}", keys[0]) ) ));
        }
    }
    match keys[1].to_uppercase().as_str() {
        "CPU" => {
            if keys[2] == "*" {
                return Ok(stats::OID::Cpu(None));
            }
            for obj in stats::cpu::CpuOID::into_enum_iter() {
                let str: &'static str = obj.to_owned().into();
                if str.eq(&keys[2].to_uppercase()) {
                    return Ok(stats::OID::Cpu(Some(obj)));
                }
            }
        },
        "MEMORY" => {
            if keys[2] == "*" {
                return Ok(stats::OID::Memory(None));
            }
            for obj in stats::memory::MemoryOID::into_enum_iter() {
                let str: &'static str = obj.to_owned().into();
                if str.eq(&keys[2].to_uppercase()) {
                    return Ok(stats::OID::Memory(Some(obj)));
                }
            }
        },
        "NETWORK" => {
            if keys[2] == "*" {
                return Ok(stats::OID::Network(None));
            }
            for obj in stats::network::NetOID::into_enum_iter() {
                let str: &'static str = obj.to_owned().into();
                if str.eq(&keys[2].to_uppercase()) {
                    return Ok(stats::OID::Network(Some(obj)));
                }
            }
        },
        "STORAGE" => {
            if keys[2] == "*" {
                return Ok(stats::OID::Storage(None));
            }
            for obj in stats::storage::StorageOID::into_enum_iter() {
                let str: &'static str = obj.to_owned().into();
                if str.eq(&keys[2].to_uppercase()) {
                    return Ok(stats::OID::Storage(Some(obj)));
                }
            }
        },
        _ => {
            return Err(Box::new( std::io::Error::new( std::io::ErrorKind::Other, format!("Unknewn subclass {} in {}", keys[1], key) ) ));
        }
    }

    return Err(Box::new( std::io::Error::new( std::io::ErrorKind::Other, format!("Item {} not found in {}", keys[2], keys[1]) ) ))
}
