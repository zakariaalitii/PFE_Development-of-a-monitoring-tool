pub mod block;
pub mod auth;
pub mod node;
pub mod cache;
pub mod template;

use sqlx::{sqlite::{SqliteConnectOptions, SqlitePool, Sqlite}, Pool};
use std::{error::Error, str::FromStr};

#[derive(Debug, Clone)]
pub struct Database {
    pub con: Pool<Sqlite>
}

impl Database {
    pub async fn new(dir: &str, master: bool) -> Result<Database, Box<dyn Error + Send + Sync>> {
        let db = Database::connect(dir).await?;
        db.create_if_not_exists(master).await?;
        Ok(db)
    }

    async fn connect(dir: &str) -> Result<Database, Box<dyn Error + Send + Sync>> {
        let opt = SqliteConnectOptions::from_str(&format!("sqlite://{}{}", dir, "/storage.db"))?
            .create_if_missing(true);

        Ok(Database { con: SqlitePool::connect_with(opt).await? })
    }

    pub async fn create_if_not_exists(&self, master: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
        if master {
            sqlx::query("pragma page_size = 4096;")
                .execute(&self.con)
                .await?;
            sqlx::query("pragma temp_store = memory;")
                .execute(&self.con)
                .await?;

            sqlx::query("create table if not exists users (
                         id integer primary key,
                         name text not null,
                         public_key blob not null,
                         permissions blob not null,
                         active bool not null,
                         last_access integer
                     )")
                .execute(&self.con)
                .await?;

            sqlx::query("create table if not exists `order` (
                         id integer primary key,
                         data blob not null,
                         hash blob not null
                     )")
                .execute(&self.con)
                .await?;

            sqlx::query("create table if not exists node (
                         id integer primary key,
                         name text not null,
                         public_key blob not null,
                         master_node bool,
                         conf text not null,
                         vote_weight integer
                     )")
                .execute(&self.con)
                .await?;

            sqlx::query("create table if not exists node_peerid (
                         node primary key,
                         peerid text not null,
                         foreign key(node) references node(id) on delete cascade
                     )")
                .execute(&self.con)
                .await?;

            sqlx::query("create table if not exists stats (
                         timestamp integer,
                         node integer,
                         data blob not null,
                         sig blob not null,
                         primary key(timestamp, node),
                         foreign key(node) references node(id) on delete cascade
                     )")
                .execute(&self.con)
                .await?;

            sqlx::query("create table if not exists config (
                         name text primary key,
                         master_conf blob,
                         node_conf blob
                     )")
                .execute(&self.con)
                .await?;
        } else {
            sqlx::query("create table if not exists node (
                         id integer primary key,
                         public_key blob not null,
                         addr text
                     )")
                .execute(&self.con)
                .await?;
            sqlx::query("create table if not exists config (
                         node primary key,
                         node_conf blob
                     )")
                .execute(&self.con)
                .await?;
        }

        
        Ok(())
    }
}

