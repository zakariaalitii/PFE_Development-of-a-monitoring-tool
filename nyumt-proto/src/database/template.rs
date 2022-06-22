use std::error::Error;
use async_stream::stream;
use futures::{ Stream, TryStreamExt };

use crate::database::Database;

#[derive(sqlx::FromRow)]
pub struct ConfigDetails {
    pub name: String,
    pub used_by: u32
}

#[derive(sqlx::FromRow)]
pub struct Config {
    pub master_conf: Vec<u8>,
    pub node_conf: Vec<u8>
}


pub async fn get_template_list<'a>(con: &'a Database) -> impl Stream<Item = Result<ConfigDetails, Box<dyn Error + Send + Sync>>> + 'a + Unpin {
    let mut v = sqlx::query_as::<_, ConfigDetails>("select config.name, (select count(*) from node where conf == config.name) as \"used_by\" from config")
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

pub async fn get_template<'a>(con: &'a Database, name: &str) -> Result<Option<Config>, Box<dyn Error + Send + Sync>> {
    let v = sqlx::query_as::<_, Config>("select master_conf, node_conf from config where name = ?1")
        .bind(name)
        .fetch_all(&con.con)
        .await?;
    for i in v {
        return Ok(Some(i));
    }
    Ok(None)
}
