use sqlx::Executor;
use std::error::Error;
use async_stream::stream;
use futures::{ Stream, TryStreamExt };

use super::Database;


#[derive(sqlx::FromRow)]
struct DbBlock {
    id: i64,
    data: Vec<u8>
}

#[derive(sqlx::FromRow)]
struct BlockId {
    id: i64
}

#[derive(sqlx::FromRow)]
struct Height {
    id: i64,
    hash: Vec<u8>
}


pub async fn get_blocks_from<'a>(con: &'a Database, height: i64) -> impl Stream<Item = Result<(u64, Vec<u8>), Box<dyn Error + Send + Sync>>> + 'a + Unpin {
    let mut v = sqlx::query_as::<_, DbBlock>("select id, data from `order` where id > ?1 order by id asc")
        .bind(height)
        .fetch(&con.con);
    Box::pin(stream! {
            loop {
                let next = v.try_next().await?;
                match next {
                    Some(data) => {
                        yield Ok((data.id as u64, data.data))
                    },
                    None => break
                }
            }
    })
}

pub async fn get_last_block_height(con: &Database) -> Result<Option<u64>, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, BlockId>("select id from `order` where id = \
                                         (select max(id) from `order`)")
            .fetch_all(&con.con).await?;
    for i in v {
        return Ok(Some(i.id as u64));
    }
    return Ok(None);
}

pub async fn get_last_block(con: &Database) -> Result<Option<(u64, Vec<u8>)>, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, Height>("select id, hash from `order` where id = \
                                         (select max(id) from `order`)")
            .fetch_all(&con.con).await?;
    for i in v {
        return Ok(Some((i.id as u64, i.hash)));
    }
    return Ok(None);
}

pub async fn add_new(con: &Database, id: u64, data: &[u8], hash: &[u8]) -> Result<(), Box<dyn Error + Send + Sync>> {
    sqlx::query("insert into `order` (id, data, hash) values (?1, ?2, ?3)")
        .bind(id as i64)
        .bind(data)
        .bind(hash)
        .execute(&con.con)
        .await?;
    Ok(())
}
