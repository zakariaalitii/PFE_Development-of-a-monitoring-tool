pub mod block;
pub mod types;
pub mod keys;
pub mod db;
pub mod api;
pub mod stats;
pub mod peer;
pub mod config;
pub mod db_calls;
pub mod database;


use lazy_static::lazy_static;

lazy_static! {
    pub static ref PROTOCOL_VERSION: f64 = 0.01;
}
