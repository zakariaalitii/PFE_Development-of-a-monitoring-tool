use tracing::{error, debug};
use systemstat::{System, Platform, saturating_sub_bytes};
use std::io::Error;
use borsh::{BorshSerialize, BorshDeserialize};
use enum_iterator::IntoEnumIterator;
use strum::IntoStaticStr;

use super::{VarValue, DataType};

#[repr(u8)]
#[derive(Debug, BorshDeserialize, BorshSerialize, serde::Serialize, serde::Deserialize, PartialEq, Clone, IntoEnumIterator, IntoStaticStr)]
pub enum MemoryOID {
    TOTAL = 0,
    USED
}

/*#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct MemUsage {
    // Memory usage is always stored in gb unit
    pub total: Option<u64>,
    pub used: Option<u64>,
}*/

pub type MemUsage = Vec<(MemoryOID, VarValue)>;

pub fn interpret_oid(oid: &Option<MemoryOID>) -> Vec<MemoryOID> {
    let mut vec = vec![];
    match oid {
        Some(v) => vec.push(v.clone()),
        None => {
            for _oid in MemoryOID::into_enum_iter() {
                vec.push(_oid);
            }
        }
    }
    vec
}

pub fn usage(sys: &System, db_struct: &Vec<MemoryOID>) -> Result<DataType, Error> {
    match sys.memory() {
        Ok(mem) => {
            debug!("Memory: {} used / {} total", mem.free.0, mem.total.0);
            let mut mem_usage: MemUsage = Vec::new();
            for i in db_struct {
                match i {
                    MemoryOID::TOTAL => mem_usage.push((MemoryOID::TOTAL, VarValue::Number(mem.total.0))),
                    MemoryOID::USED => mem_usage.push((MemoryOID::USED, VarValue::Number(saturating_sub_bytes(mem.total, mem.free).0)))
                }
            }
            return Ok(DataType::Memory(mem_usage));
        },
        Err(e) => {
            error!("Memory: error: {}", e);
            return Err(e);
        },
    }
}
