use std::error::Error;
use deadpass::{
    Database, Archive, Serialize, Deserialize, SealedSecret,
    SecretVal, SecretVec,
    ComposedKey
};
use nyumt_proto::keys::PublicKey;

pub use deadpass::DbOptions;

#[derive(Debug, Archive, Serialize, Deserialize, SealedSecret)]
pub enum KeyPair {
    ED25519(SecretVal<[u8; 64]>)
}

impl KeyPair {
    pub fn generate() -> KeyPair {
        let mut seed: rand_1::rngs::StdRng = rand_1::SeedableRng::from_entropy();
        let glue_keypair = ed25519_dalek::Keypair::generate(&mut seed);

        KeyPair::ED25519(SecretVal(glue_keypair.to_bytes()))
    }

    pub fn public(&self) -> PublicKey {
        match self {
            KeyPair::ED25519(v) => PublicKey::ED25519(v.0[32..].try_into().expect("Should be 32 bits key"))
        }
    }
    pub fn to_keypair(&self) -> Result<libp2p::identity::Keypair, Box<dyn Error>> {
        match self {
            KeyPair::ED25519(v) => Ok(libp2p::identity::Keypair::Ed25519(libp2p::identity::ed25519::Keypair::decode(&mut v.0.clone())?))
        }
    }
}

impl std::fmt::Display for KeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", format!("{}", self.public()))
    }
} 

#[derive(Debug, Archive, Serialize, Deserialize, SealedSecret)]
pub struct Session {
    /// Name of session
    pub name: SecretVal<String>,
    /// Host that we connect to (ie. master node)
    pub host: SecretVal<String>,
    /// Keypair
    pub keys: KeyPair,
}

#[derive(Debug, Archive, Serialize, Deserialize, SealedSecret)]
pub struct Sessions(pub SecretVec<Session>);

#[derive(Debug, Archive, Serialize, Deserialize, SealedSecret)]
pub struct DB {
    pub session: Sessions
}

impl DB {
    fn new() -> DB {
        DB {
            session: Sessions(SecretVec::new())
        }
    }
}

pub struct Storage {
    db: Database<DB>,
}

impl Storage {
    pub fn new<'a>(db_path: &str, pass: &[u8], keyfile_path: Option<&'a str>, options: Option<DbOptions>) -> Result<Storage, Box<dyn Error>> {
        let keys = match keyfile_path {
            Some(v) => ComposedKey::new(pass, v)?,
            None    => ComposedKey::from_pass(pass)
        };
        let db = Database::new(DB::new(), db_path, keys, options)?;
        Ok(Storage { db })
    }

    pub fn open<'a>(db_path: &str, pass: &[u8], keyfile_path: Option<&'a str>) -> Result<Storage, Box<dyn Error>> {
        let keys = match keyfile_path {
            Some(v) => ComposedKey::new(pass, v)?,
            None    => ComposedKey::from_pass(pass)
        };
        let db = Database::open(db_path, keys)?;
        Ok(Storage { db })
    }

    #[inline]
    pub fn save(&mut self) -> Result<(), Box<dyn Error>> {
        self.db.save()
    }

    #[inline]
    pub fn get<'a>(&'a self) -> &'a DB {
        self.db.get()
    }

    #[inline]
    pub fn get_mut<'a>(&'a mut self) -> &'a mut DB {
        self.db.get_mut()
    }
}
