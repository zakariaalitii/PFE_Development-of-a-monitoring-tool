use sqlx::{Pool, Sqlite};
use std::error::Error;

#[derive(sqlx::FromRow, Debug)]
struct Exists {
    exist: u32
}

pub async fn entity_with_pubkey_exists(con: &Pool<Sqlite>, pubkey: &[u8]) -> Result<bool, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, Exists>(r#"select 1 as "exist" where exists (select 1 from users where public_key = ?1 and active = true)
                                           or exists (select 1 from node where public_key = ?1)"#)
        .bind(pubkey)
        .fetch_all(con).await?;
    for _ in v {
        return Ok(true);
    }
    Ok(false)
}

pub async fn master_with_pubkey_exists(con: &Pool<Sqlite>, pubkey: &[u8]) -> Result<bool, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, Exists>(r#"select 1 as "exist" from node where public_key = ?1 and master_node = true"#)
        .bind(pubkey)
        .fetch_one(con).await?;
    if v.exist >= 1 {
        return Ok(true);
    }
    Ok(false)
}

pub async fn node_with_pubkey_exists(con: &Pool<Sqlite>, pubkey: &[u8]) -> Result<bool, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, Exists>(r#"select 1 as "exist" from node where public_key = ?1"#)
        .bind(pubkey)
        .fetch_one(con).await?;
    if v.exist >= 1 {
        return Ok(true);
    }
    Ok(false)
}

pub async fn config_exists(con: &Pool<Sqlite>, name: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, Exists>(r#"select 1 as "exist" from config where name = ?1"#)
        .bind(name)
        .fetch_one(con).await?;
    Ok(v.exist > 0)
}

pub async fn config_used(con: &Pool<Sqlite>, name: &str) -> Result<bool, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, Exists>(r#"select 1 as "exist" from node where conf = ?1"#)
        .bind(name)
        .fetch_one(con).await?;
    Ok(v.exist > 0)
}
