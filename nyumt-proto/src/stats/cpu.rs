use tracing::{error, debug};
use systemstat::{System, Platform};
use std::{time::Duration, thread, io::Error};
use borsh::{BorshSerialize, BorshDeserialize};
use enum_iterator::IntoEnumIterator;
use strum::IntoStaticStr;

use super::{VarValue, DataType};

#[repr(u8)]
#[derive(Debug, BorshDeserialize, BorshSerialize, serde::Serialize, serde::Deserialize, PartialEq, Clone, IntoEnumIterator, IntoStaticStr)]
pub enum CpuOID {
    TOTAL = 0,
    USER,
    NICE,
    SYSTEM,
    INT,
    IDLE
}

pub type CpuUsage = Vec<(CpuOID, VarValue)>;

/*#[derive(Debug, BorshDeserialize, BorshSerialize)]
pub struct CpuUsage {
    pub total: Option<f32>, // cpu used in total
    pub user: Option<f32>, // cpu used in user space
    pub nice: Option<f32>, // cpu usage on processes with nice value
    pub system: Option<f32>, // cpu spent in kernel
    pub int: Option<f32>, // interrupt both software and hardware
    pub idle: Option<f32>, // cpu idle
}*/

pub fn interpret_oid(oid: &Option<CpuOID>) -> Vec<CpuOID> {
    let mut vec = vec![];
    match oid {
        Some(v) => vec.push(v.clone()),
        None => {
            for _oid in CpuOID::into_enum_iter() {
                vec.push(_oid);
            }
        }
    }
    vec
}

pub fn usage(sys: &System, db_struct: &Vec<CpuOID>) -> Result<DataType, Error> {
    match sys.cpu_load_aggregate() {
        Ok(cpu) => {
            thread::sleep(Duration::from_secs(1));
            let cpu = match cpu.done() {
                Ok(v)  => v,
                Err(e) => {
                    error!("CPU load: error: {}", e);
                    return Err(e);
                },
            };
            debug!("CPU load: {}% user, {}% nice, {}% system, {}% intr, {}% idle ",
                cpu.user, cpu.nice, cpu.system, cpu.interrupt, cpu.idle);
            let mut cpu_usage: CpuUsage = Vec::new();
            for i in db_struct {
                match i {
                    CpuOID::TOTAL => cpu_usage.push((CpuOID::TOTAL, VarValue::RealNumber(100.0 - (cpu.idle * 100.0)))),
                    CpuOID::USER => cpu_usage.push((CpuOID::USER, VarValue::RealNumber(cpu.user * 100.0))),
                    CpuOID::NICE => cpu_usage.push((CpuOID::NICE, VarValue::RealNumber(cpu.nice * 100.0))),
                    CpuOID::SYSTEM => cpu_usage.push((CpuOID::SYSTEM, VarValue::RealNumber(cpu.system * 100.0))),
                    CpuOID::INT => cpu_usage.push((CpuOID::INT, VarValue::RealNumber(cpu.interrupt * 100.0))),
                    CpuOID::IDLE => cpu_usage.push((CpuOID::IDLE, VarValue::RealNumber(cpu.idle * 100.0)))
                }
            }

            return Ok(DataType::Cpu(cpu_usage));
        },
        Err(e) => {
            error!("CPU load: error: {}", e);
            return Err(e);
        },
    }
}
