use tracing::{error, debug};
use systemstat::{System, Platform};
use std::io::Error;
use borsh::{BorshSerialize, BorshDeserialize};
use enum_iterator::IntoEnumIterator;
use strum::IntoStaticStr;

use super::{VarValue, DataType};


#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct StorageConstDbVal {
    // Collection of every interface name and it's id
    pub storage_name_id: Vec<String>,
    pub storage_filesystem_type: Vec<String>
}

impl StorageConstDbVal {
    pub fn new() -> StorageConstDbVal {
        StorageConstDbVal {
            storage_name_id: vec![],
            storage_filesystem_type: vec![]
        }
    }
}

#[repr(u8)]
#[derive(Debug, BorshDeserialize, BorshSerialize, serde::Serialize, serde::Deserialize, PartialEq, Clone, IntoEnumIterator, IntoStaticStr)]
pub enum StorageOID {
    NAME,
    FILESYSTEM_TYPE,
    FREE,
    TOTAL,
    TOTAL_NODES,
    FREE_NODES,
}

pub type Storage = Vec<Vec<(StorageOID, VarValue)>>;

/*
#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct StorageList(pub Vec<Storage>);

#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct Storage {
    pub name: StorageId,
    pub free: Option<u64>,
    pub total: Option<u64>
}*/

pub fn interpret_oid(oid: &Option<StorageOID>) -> Vec<StorageOID> {
    let mut vec = vec![];
    match oid {
        Some(v) => vec.push(v.clone()),
        None => {
            for _oid in StorageOID::into_enum_iter() {
                vec.push(_oid);
            }
        }
    }
    vec
}

pub async fn usage(sys: &System, db_struct: &Vec<StorageOID>) -> Result<DataType, Error> {
    match sys.mounts() {
        Ok(mounts) => {
            let mut list: Storage = Vec::new();
            for mount in mounts.iter() {
                if !mount.fs_mounted_from.starts_with("/dev") {
                    continue;
                }
                let (dev_name, fs_type) = (VarValue::Str(mount.fs_mounted_from.clone()), VarValue::Str(mount.fs_type.clone()));
                if device_exists(&list, &dev_name){
                    continue;
                }

                debug!("{} (available {} of {})", mount.fs_mounted_from, mount.avail.0, mount.total.0);
                
                let mut dev: Vec<(StorageOID, VarValue)> = Vec::new();
                for i in db_struct {
                    match i {
                        StorageOID::NAME => dev.push((StorageOID::NAME, dev_name.clone())),
                        StorageOID::FILESYSTEM_TYPE => dev.push((StorageOID::FILESYSTEM_TYPE, fs_type.clone())),
                        StorageOID::FREE => dev.push((StorageOID::FREE, VarValue::Number(mount.avail.0))),
                        StorageOID::TOTAL => dev.push((StorageOID::TOTAL, VarValue::Number(mount.total.0))),
                        StorageOID::TOTAL_NODES => dev.push((StorageOID::TOTAL_NODES, VarValue::Number(mount.files_total as u64))),
                        StorageOID::FREE_NODES => dev.push((StorageOID::FREE_NODES, VarValue::Number(mount.files_avail as u64))),
                    }
                }

                list.push(dev);
            }
            return Ok(DataType::Storage(list));
        }
        Err(e) => {
            error!("Mounts: error: {}", e);
            return Err(e);
        },
    }
}

fn device_exists(list: &Storage, name: &VarValue) -> bool {
    for mount in list {
        for val in mount {
            if val.0 == StorageOID::NAME && name.eq(&val.1) {
                return true;
            }
        }
    }
    false
}
