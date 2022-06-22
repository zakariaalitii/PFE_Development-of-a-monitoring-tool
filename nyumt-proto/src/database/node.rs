use std::error::Error;
use async_stream::stream;
use futures::{ Stream, TryStreamExt };
use async_compression::tokio::write;
use tokio::io::AsyncWriteExt;
use tracing::error;

use super::Database;

#[derive(sqlx::FromRow)]
struct StatsCount {
    count: u32
}

#[derive(sqlx::FromRow)]
struct MasterNodeCount {
    count: u32
}

#[derive(sqlx::FromRow)]
struct Stats {
    node: u32,
    data: Vec<u8>,
    sig: Vec<u8>,
    timestamp: i64
}

#[derive(sqlx::FromRow)]
struct NodeStats {
    data: Vec<u8>,
    timestamp: i64
}

#[derive(sqlx::FromRow)]
struct PeerPk {
    public_key: Vec<u8>
}

#[derive(sqlx::FromRow)]
struct Config {
    conf: String
}

#[derive(sqlx::FromRow)]
struct IsMasterNode {
    master_node: bool
}

#[derive(sqlx::FromRow)]
struct NodeId {
    id: u32
}

#[derive(sqlx::FromRow)]
pub struct Node {
    pub id: u32,
    pub name: String,
    pub public_key: Vec<u8>,
    pub master_node: bool,
    pub conf: String
}

#[derive(sqlx::FromRow)]
pub struct MasterNode {
    pub public_key: Vec<u8>
}

#[derive(sqlx::FromRow)]
pub struct MasterNodeAddr {
    pub addr: Option<String>
}

#[derive(sqlx::FromRow)]
struct NodeConfig {
    node_conf: Vec<u8>
}

#[derive(sqlx::FromRow)]
struct StatsTimestamp {
    timestamp: i64
}

pub async fn get_nodes<'a>(con: &'a Database) -> impl Stream<Item = Result<Node, Box<dyn Error + Send + Sync>>> + 'a + Unpin {
    let mut v = sqlx::query_as::<_, Node>("select id, name, public_key, master_node, conf from node")
        .fetch(&con.con);
    Box::pin(stream! {
            loop {
                let next = v.try_next().await?;
                match next {
                    Some(data) => {
                        yield Ok(data)
                    },
                    None => break
                }
            }
    })
}

pub async fn get_master_nodes<'a>(con: &'a Database) -> impl Stream<Item = Result<MasterNode, Box<dyn Error + Send + Sync>>> + 'a + Unpin {
    let mut v = sqlx::query_as::<_, MasterNode>("select public_key from node where master_node = true")
        .fetch(&con.con);
    Box::pin(stream! {
            loop {
                let next = v.try_next().await?;
                match next {
                    Some(data) => {
                        yield Ok(data)
                    },
                    None => break
                }
            }
    })
}

pub async fn node_get_master_nodes_addr<'a>(con: &'a Database) -> impl Stream<Item = Result<MasterNodeAddr, Box<dyn Error + Send + Sync>>> + 'a + Unpin {
    let mut v = sqlx::query_as::<_, MasterNodeAddr>("select addr from node")
        .fetch(&con.con);
    Box::pin(stream! {
            loop {
                let next = v.try_next().await?;
                match next {
                    Some(data) => {
                        yield Ok(data)
                    },
                    None => break
                }
            }
    })
}

pub async fn store_stats(con: &Database, node_pk: &[u8], data: &[u8], sig: &[u8], timestamp: u64) -> Result<(), Box<dyn Error + Send + Sync>> {

    sqlx::query("insert into stats (node, data, sig, timestamp) values ((select id from node where public_key = ?4), ?1, ?2, ?3)")
        .bind(data)
        .bind(sig)
        .bind(timestamp as i64)
        .bind(node_pk)
        .execute(&con.con)
        .await?;

    Ok(())
}

pub async fn store_stats_with_id(con: &Database, id: u32, data: &[u8], sig: &[u8], timestamp: u64) -> Result<(), Box<dyn Error + Send + Sync>> {
    sqlx::query("insert into stats (node, data, sig, timestamp) values (?4, ?1, ?2, ?3)")
        .bind(data)
        .bind(sig)
        .bind(timestamp as i64)
        .bind(id)
        .execute(&con.con)
        .await?;

    Ok(())
}

pub async fn poll_stats_ready(con: &Database, timestamp: u64) -> Result<bool, Box<dyn Error + Send + Sync>> {
    let pk = sqlx::query_as::<_, StatsCount>(r#"select count(*) as "count" from stats where timestamp > ?1"#)
        .bind(timestamp as i64)
        .fetch_one(&con.con).await?;
    if pk.count >= 1 {
        return Ok(true)
    }
    Ok(false)
}

pub async fn get_stats_from<'a>(con: &'a Database, timestamp: u64) -> impl Stream<Item = Result<(u32, Vec<u8>, Vec<u8>, u64), Box<dyn Error + Send + Sync>>> + 'a + Unpin {
    let mut v = sqlx::query_as::<_, Stats>("select node, data, sig, timestamp from stats where timestamp > ?1")
        .bind(timestamp as i64)
        .fetch(&con.con);
    Box::pin(stream! {
        loop {
            let next = v.try_next().await?;
            match next {
                Some(data) => {
                    yield Ok((data.node, data.data, data.sig, data.timestamp as u64))
                },
                None => break
            }
        }
    })
}

pub async fn get_node_stats_from<'a>(con: &'a Database, pk: Vec<u8>, timestamp: u64) -> impl Stream<Item = Result<(Vec<u8>, u64), Box<dyn Error + Send + Sync>>> + 'a + Unpin {
    let mut v = sqlx::query_as::<_, NodeStats>("select data, timestamp from stats where node = (select id from node where public_key = ?1) and timestamp > ?2")
        .bind(pk)
        .bind(timestamp as i64)
        .fetch(&con.con);
    Box::pin(stream! {
        loop {
            let next = v.try_next().await;
            match next? {
                Some(data) => {
                    yield Ok((data.data, data.timestamp as u64))
                },
                None => break
            }
        }
    })
}

pub async fn get_last_stats_timestamp(con: &Database) -> Result<Option<u64>, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, StatsTimestamp>("select max(timestamp) as \"timestamp\" from stats")
            .fetch_all(&con.con).await?;
    for i in v {
        return Ok(Some(i.timestamp as u64));
    }
    return Ok(None);
}

pub async fn get_min_stats_timestamp_in_limit(con: &Database, pk: &[u8], limit: u32) -> Result<Option<u64>, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, StatsTimestamp>("select min(timestamp) as \"timestamp\" from (select timestamp from stats where node = (select id from node where public_key = ?2)
                                                 order by timestamp desc limit ?1)")
        .bind(limit)
        .bind(pk)
        .fetch_all(&con.con).await?;
    for i in v {
        return Ok(Some(i.timestamp as u64));
    }
    return Ok(None);
}

pub async fn master_node_count(con: &Database) -> Result<u32, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, MasterNodeCount>(r#"select count(*) as "count" from node where master_node = true"#)
        .fetch_one(&con.con).await?;
    Ok(v.count)
}

pub async fn node_master_node_count(con: &Database) -> Result<u32, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, MasterNodeCount>(r#"select count(*) as "count" from node"#)
        .fetch_one(&con.con).await?;
    Ok(v.count)
}

pub async fn get_node_id(con: &Database, pk: &[u8]) -> Result<Option<u32>, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, NodeId>("select id from node where public_key = ?1")
        .bind(pk)
        .fetch_all(&con.con)
        .await?;
    for i in v {
        return Ok(Some(i.id));
    }
    return Ok(None);
}

pub async fn update_peerid(con: &Database, pk: &[u8], peerid: String) -> Result<(), Box<dyn Error + Send + Sync>> {
    sqlx::query("insert into node_peerid (node, peerid) values ((select id from node where public_key = ?1), ?2) on conflict(node) do update set peerid = ?2 \
                 where exists (select 1 from node where public_key = ?1)")
        .bind(pk)
        .bind(peerid)
        .execute(&con.con)
        .await?;
    Ok(())
}

pub async fn get_peer_pk(con: &Database, peerid: &str) -> Result<Option<Vec<u8>>, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, PeerPk>("select public_key from node where id = (select node from node_peerid where peerid = ?1)")
        .bind(peerid)
        .fetch_all(&con.con).await?;
    for i in v {
        return Ok(Some(i.public_key));
    }
    return Ok(None);
}

pub async fn is_peer_master_node(con: &Database, peerid: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, IsMasterNode>("select master_node from node where id = (select node from node_peerid where peerid = ?1)")
        .bind(peerid)
        .fetch_all(&con.con).await?;
    for i in v {
        return Ok(i.master_node);
    }
    return Ok(false);
}

pub async fn is_node_master(con: &Database, pk: &[u8]) -> Result<bool, Box<dyn Error>> {
    let v = sqlx::query_as::<_, IsMasterNode>("select master_node from node where public_key = ?1")
        .bind(pk)
        .fetch_all(&con.con).await?;
    for i in v {
        return Ok(i.master_node);
    }
    return Ok(false);
}

pub async fn get_peer_master_node_pk(con: &Database, peerid: &str) -> Result<Option<Vec<u8>>, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, PeerPk>("select public_key from node where id = (select node from node_peerid where peerid = ?1) and master_node = true")
        .bind(peerid)
        .fetch_all(&con.con).await?;
    for i in v {
        return Ok(Some(i.public_key));
    }
    return Ok(None);
}

pub async fn add_master_nodes(con: &Database, masters: &[(Vec<u8>, Option<String>)]) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut tx = con.con.begin().await?;
    sqlx::query("delete from node")
        .execute(&mut tx)
        .await?;
    for master in masters {
        sqlx::query("insert into node (public_key, addr) values (?1, ?2)")
            .bind(&master.0)
            .bind(&master.1)
            .execute(&mut tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn get_peer_conf_name(con: &Database, peerid: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, Config>("select conf from node where id = (select node from node_peerid where peerid = ?1)")
        .bind(peerid)
        .fetch_one(&con.con).await?;
    return Ok(v.conf);
}

pub async fn get_peer_conf(con: &Database, peerid: &str) -> Result<Vec<u8>, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, NodeConfig>("select node_conf from config where config.name = (select conf from node where node.id = (select node from node_peerid where peerid = ?1))")
        .bind(peerid)
        .fetch_one(&con.con).await?;
    return Ok(v.node_conf);
}
