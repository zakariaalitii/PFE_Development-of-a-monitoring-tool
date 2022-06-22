use std::error::Error;

use super::Database;


#[derive(sqlx::FromRow)]
struct Config {
    name: String,
    master_conf: Vec<u8>,
    node_conf: Vec<u8>
}

#[derive(sqlx::FromRow)]
struct NodeConfig {
    node_conf: Vec<u8>
}


pub async fn load_local_master_node_conf(con: &Database, pk: &[u8]) -> Result<Option<(String, Vec<u8>, Vec<u8>)>, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, Config>("select name, master_conf, node_conf from config where name = (select conf from node where public_key = ?1)")
        .bind(pk)
        .fetch_all(&con.con)
        .await?;
    for i in v {
        return Ok(Some((i.name, i.master_conf, i.node_conf)));
    }
    return Ok(None);
}

pub async fn update_local_node_conf(con: &Database, conf: &[u8]) -> Result<(), Box<dyn Error + Send + Sync>> {
    sqlx::query("insert into config (node, node_conf) values (0, ?1) on conflict(node) do update set node_conf = ?1 where node = 0")
        .bind(conf)
        .execute(&con.con)
        .await?;
    Ok(())
}

pub async fn load_local_node_conf(con: &Database) -> Result<Option<Vec<u8>>, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, NodeConfig>("select node_conf from config where node = 0")
        .fetch_all(&con.con)
        .await?;
    for i in v {
        return Ok(Some(i.node_conf));
    }
    return Ok(None);
}

pub async fn get_node_conf(con: &Database, name: &str) -> Result<Option<(String, Vec<u8>, Vec<u8>)>, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, Config>("select name, master_conf, node_conf from config where name = ?1")
        .bind(name)
        .fetch_all(&con.con)
        .await?;
    for i in v {
        return Ok(Some((i.name, i.master_conf, i.node_conf)));
    }
    return Ok(None);
}
