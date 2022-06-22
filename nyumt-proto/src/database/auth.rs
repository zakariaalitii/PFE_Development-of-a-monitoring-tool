use std::error::Error;
use async_stream::stream;
use futures::{ Stream, TryStreamExt };

use crate::database::Database;

#[derive(sqlx::FromRow)]
struct PubkeyExists {
    exist: u32
}

#[derive(sqlx::FromRow)]
pub struct UserDetails {
    pub name: String,
    pub permissions: Vec<u8>,
    pub last_access: i64
}

#[derive(sqlx::FromRow)]
pub struct User {
    pub id: u32,
    pub name: String,
    pub permissions: Vec<u8>,
    pub last_access: i64,
    pub public_key: Vec<u8>,
    pub active: bool
}

pub async fn user_with_pubkey_exists(con: &Database, pubkey: &[u8]) -> Result<bool, Box<dyn Error>> {
    let pk = sqlx::query_as::<_, PubkeyExists>(r#"select count(*) as "exist" from users where public_key = ?1 and active = true"#)
        .bind(pubkey)
        .fetch_one(&con.con).await?;
    if pk.exist >= 1 {
        return Ok(true)
    }
    Ok(false)
}

pub async fn get_user_with_pubkey(con: &Database, pubkey: &[u8]) -> Result<UserDetails, Box<dyn Error + Send + Sync>> {
    Ok(sqlx::query_as::<_, UserDetails>(r#"select name, permissions, last_access from users where public_key = ?1 and active = true"#)
        .bind(pubkey)
        .fetch_one(&con.con).await?)
}

pub async fn get_users<'a>(con: &'a Database) -> impl Stream<Item = Result<User, Box<dyn Error + Send + Sync>>> + 'a + Unpin {
    let mut v = sqlx::query_as::<_, User>("select id, name, permissions, last_access, public_key, active from users")
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
