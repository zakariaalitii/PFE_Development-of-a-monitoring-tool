use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use std::{error::Error, str::FromStr};

pub use nyumt_proto::database::Database;

use crate::NYUMT_DIR_STR;

pub struct ClientDatabase {}

impl ClientDatabase {
    pub async fn new(session_name: &str) -> Result<Database, Box<dyn Error + Send + Sync>> {
        let db = ClientDatabase::connect(session_name).await?;
        ClientDatabase::create_if_not_exists(&db).await?;
        Ok(db)
    }

    async fn connect(session_name: &str) -> Result<Database, Box<dyn Error + Send + Sync>> {
        let opt = SqliteConnectOptions::from_str(&format!("sqlite://{}/{}.db", *NYUMT_DIR_STR, session_name))?
            .create_if_missing(true);

        Ok(Database { con: SqlitePool::connect_with(opt).await? })
    }

    pub async fn create_if_not_exists(db: &Database) -> Result<(), Box<dyn Error + Send + Sync>> {
        sqlx::query("pragma page_size = 4096;")
            .execute(&db.con)
            .await?;
        sqlx::query("pragma temp_store = memory;")
            .execute(&db.con)
            .await?;

        sqlx::query("create table if not exists users (
                     id integer primary key,
                     name text not null,
                     public_key blob not null,
                     permissions blob not null,
                     active bool not null,
                     last_access integer
                 )")
            .execute(&db.con)
            .await?;

        sqlx::query("create table if not exists `order` (
                     id integer primary key,
                     data blob not null,
                     hash blob not null
                 )")
            .execute(&db.con)
            .await?;

        sqlx::query("create table if not exists node (
                     id integer primary key,
                     name text not null,
                     public_key blob not null,
                     master_node bool,
                     conf text not null,
                     vote_weight integer
                 )")
            .execute(&db.con)
            .await?;

        sqlx::query("create table if not exists stats (
                     timestamp integer,
                     node integer,
                     data blob not null,
                     sig blob not null,
                     primary key(timestamp, node),
                     foreign key(node) references node(id) on delete cascade
                 )")
            .execute(&db.con)
            .await?;

        sqlx::query("create table if not exists config (
                     name text primary key,
                     master_conf blob,
                     node_conf blob
                 )")
            .execute(&db.con)
            .await?;
        
        sqlx::query("create table if not exists constdb_cache (
             id integer primary key,
             constdb blob not null
         )")
            .execute(&db.con)
            .await?;
        
        Ok(())
    }
}
