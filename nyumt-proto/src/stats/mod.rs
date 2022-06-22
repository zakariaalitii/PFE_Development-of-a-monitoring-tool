pub mod cpu;
pub mod memory;
pub mod storage;
pub mod network;

pub use enum_iterator;
pub use strum;
pub use systemstat::{System, Platform};
pub use self::{
    cpu::CpuUsage,
    memory::MemUsage,
    storage::Storage,
    network::NetworkUsage
};

use std::{io::Error, sync::Arc};
use chrono::Utc;
use tracing::debug;
use borsh::{BorshSerialize, BorshDeserialize};
use tokio::sync::RwLock;
use enum_iterator::IntoEnumIterator;

/// bytes size for 1 gigabyte
pub const GB: f64 = 1_000_000_000.0;

#[derive(Debug)]
pub enum VarType {
    Mandatory,
    Volatile,
    Stable
}

#[derive(Debug)]
pub enum VarValueType {
    Str,
    Int,
    Number,
    RealNumber,
    Bool,
    VecStr
}

#[derive(Debug, BorshDeserialize, BorshSerialize, Clone, PartialEq)]
pub enum VarValue {
    Str(String),
    Int(u32),
    Number(u64),
    RealNumber(f32),
    Bool(bool),
    VecStr(Vec<String>)
}

impl ToString for VarValue {
    fn to_string(&self) -> String {
        match self {
            VarValue::Str(v) => v.to_owned(),
            VarValue::Int(v) => v.to_string(),
            VarValue::Number(v) => v.to_string(),
            VarValue::RealNumber(v) => v.to_string(),
            VarValue::Bool(v) => v.to_string(),
            VarValue::VecStr(_) => "".to_owned()
        }
    }
}

pub enum EventCapture {
    Number(OID, NumberEvent),
    Bool(OID, BoolEvent)
}

pub enum NumberEvent {
    Above(i64),
    Below(i64)
}

pub enum BoolEvent {
    IsTrue(bool),
    IsFalse(bool)
}

#[repr(u8)]
#[derive(Debug, BorshSerialize, BorshDeserialize, serde::Serialize, serde::Deserialize)]
pub enum OID {
    Cpu(Option<cpu::CpuOID>),
    Memory(Option<memory::MemoryOID>),
    Network(Option<network::NetOID>),
    Storage(Option<storage::StorageOID>),
    All
}

#[derive(Debug, BorshDeserialize, BorshSerialize, Clone, serde::Serialize, serde::Deserialize)]
pub struct DbStruct {
    pub cpu: Vec<cpu::CpuOID>,
    pub memory: Vec<memory::MemoryOID>,
    pub network: Vec<network::NetOID>,
    pub storage: Vec<storage::StorageOID>
}

impl DbStruct {
    pub fn new() -> DbStruct {
        DbStruct {
            cpu: vec![],
            memory: vec![],
            network: vec![],
            storage: vec![]
        }
    }

    pub fn generate() -> DbStruct {
        let mut db_struct = DbStruct::new();
        for i in cpu::CpuOID::into_enum_iter() {
            db_struct.cpu.push(i);
        }
        for i in memory::MemoryOID::into_enum_iter() {
            db_struct.memory.push(i);
        }
        for i in network::NetOID::into_enum_iter() {
            db_struct.network.push(i);
        }
        for i in storage::StorageOID::into_enum_iter() {
            db_struct.storage.push(i);
        }

        db_struct
    }

    pub fn from_oid(oid: &OID) -> DbStruct {
        let mut cpu     = vec![];
        let mut memory  = vec![];
        let mut network = vec![];
        let mut storage = vec![];

        match oid {
            OID::Cpu(v) => cpu = cpu::interpret_oid(v),
            OID::Memory(v) => memory = memory::interpret_oid(v),
            OID::Network(v) => network = network::interpret_oid(v),
            OID::Storage(v) => storage = storage::interpret_oid(v),
            OID::All => {
                cpu     = cpu::interpret_oid(&None);
                memory  = memory::interpret_oid(&None);
                network = network::interpret_oid(&None);
                storage = storage::interpret_oid(&None);
            }
        }

        DbStruct {
            cpu,
            memory,
            network,
            storage
        }
    }
}

#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub enum Trigger {
    Memory,
    Network,
    Storage
}

#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct ConstDb {
    pub network: network::NetConstDbVal,
    pub storage: storage::StorageConstDbVal
}

impl ConstDb {
    pub fn new() -> ConstDb {
        ConstDb {
            network: network::NetConstDbVal::new(),
            storage: storage::StorageConstDbVal::new()
        }
    }
}

/*#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct Report1 {
    pub cpu: Option<cpu::CpuUsage>,
    pub memory: Option<memory::MemUsage>,
    pub network: Option<network::NetworkUsage>,
    pub storage: Option<storage::StorageList>,
    pub timestamp: u64
}*/

#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub enum DataType {
    Cpu(cpu::CpuUsage),
    Memory(memory::MemUsage),
    Network(network::NetworkUsage),
    Storage(storage::Storage)
}

#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct Report {
    pub data: Vec<DataType>,
    pub timestamp: u64
}

pub async fn generate_report(sys: &Arc<System>, db_struct: &Arc<DbStruct>) -> Result<Report, Error> {
    let time   = Utc::now();

    let mut data: Vec<DataType> = Vec::new();
    let mut futures             = Vec::new();

    if !db_struct.cpu.is_empty() {
        let sys_clone       = sys.clone();
        let db_struct_clone = db_struct.clone();
        futures.push(tokio::task::spawn_blocking(move || {
            cpu::usage(&sys_clone, &db_struct_clone.cpu)
        }));
    }
    if !db_struct.memory.is_empty() {
        let sys_clone       = sys.clone();
        let db_struct_clone = db_struct.clone();
        futures.push(tokio::spawn(async move { memory::usage(&sys_clone, &db_struct_clone.memory) }))
    }
    if !db_struct.network.is_empty() {
        let sys_clone       = sys.clone();
        let db_struct_clone = db_struct.clone();
        futures.push(tokio::spawn(async move { network::usage(&sys_clone, &db_struct_clone.network).await }))
    }
    if !db_struct.storage.is_empty() {
        let sys_clone       = sys.clone();
        let db_struct_clone = db_struct.clone();
        futures.push(tokio::spawn(async move { storage::usage(&sys_clone, &db_struct_clone.storage).await }))
    }

    for future in futures {
        data.push(future.await??);
    }


    let report = Report {
        data,
        timestamp: time.timestamp() as u64
    };
    debug!("New monitoring report generated on {}", time);

    Ok(report)
}
