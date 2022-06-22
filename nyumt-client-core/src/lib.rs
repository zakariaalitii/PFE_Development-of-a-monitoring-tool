pub mod storage;
pub mod client;
pub mod db;
pub mod order;
pub mod stats;

use lazy_static::lazy_static;
pub use deadpass;
use tracing::error;
use std::fs;
use tokio::time::Duration;

lazy_static! {
    pub static ref HOME_DIR: std::path::PathBuf = match std::env::var("NYUMT_HOME") {
        Ok(v) => std::path::PathBuf::from(v),
        Err(_) => match dirs::home_dir() {
            Some(v) => v,
            None    => panic!("Platform not supported")
        }
    };
    pub static ref NYUMT_DIR: std::path::PathBuf = std::path::PathBuf::from(format!("{}/.nyumt/", HOME_DIR.to_str().unwrap()));
    pub static ref NYUMT_DIR_STR: &'static str = NYUMT_DIR.to_str().unwrap();

    static ref POLL_TIME: Duration = Duration::from_millis(500);
}

pub fn init() {
    let conf_dir_clone = NYUMT_DIR.clone();
    if !conf_dir_clone.exists() {
        match fs::create_dir(&conf_dir_clone) {
            Ok(_)  => (),
            Err(e) => {
                error!("Error creating nyumt with path {}: {}", conf_dir_clone.to_str().unwrap(), e);
            }
        }
    }
}
