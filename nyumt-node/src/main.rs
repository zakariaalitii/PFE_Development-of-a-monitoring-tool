mod utils;
mod master;
mod master_serve;
mod node;
mod database;
mod config;
mod info;

use std::{fs, io::Write};
use dirs;
use clap::{Command, Arg};
use std::error::Error;
use tracing_subscriber;
use tracing::{error, info, warn};
use serde::{Serialize, Deserialize};
use lazy_static::lazy_static;
use tokio::{
    runtime::Runtime,
    sync::mpsc::{self, Sender, Receiver},
    time::Duration
};

use crate::utils::keys::{ save_keypair, generate_keypair, LocalPeerKey, get_keypair };
use crate::master::types::Request;
use crate::utils::keys::PublicKey;

lazy_static! {
    pub static ref HOME_DIR: std::path::PathBuf = match std::env::var("NYUMT_HOME") {
        Ok(v) => std::path::PathBuf::from(v),
        Err(_) => match dirs::home_dir() {
            Some(v) => v,
            None    => panic!("Platform not supported")
        }
    };
    pub static ref NYUMT_DIR: std::path::PathBuf = std::path::PathBuf::from(format!("{}/.nyumt/", HOME_DIR.to_str().unwrap()));
    pub static ref NYUMT_DIR_STR: &'static str = NYUMT_DIR.to_str().unwrap();

    static ref POLL_TIME: Duration = Duration::from_millis(500);
    static ref CLIENT_POLL_TIME: Duration = Duration::from_millis(4000);
}

pub enum Stop {
    SoftRestart,
    Panic
}

pub struct DaemonSettings {
    auto_discovery: bool
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JoinPeer {
    pub addr: String,
    pub publickey: String
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Settings {
    master_node: bool,
    timeout: usize,
    client_port: u16,
    local_peer_port: u16,
    peers: Vec<JoinPeer>
}

async fn arg_parse() -> Result<(Settings, Option<DaemonSettings>, LocalPeerKey), Box<dyn Error>> {
    let glob_args = Command::new("Nyumt node daemon")
    .version("1.0")
    .author("zesty")
    .subcommand(Command::new("daemon")
                .about("Launch node daemon")
                .arg(Arg::new("auto_discovery")
                    .short('a')
                    .long("auto-discovery")
                    .help("Allow auto discovery for other nodes, useful when nodes have dynamic ip address, node only join knewn nodes (mdns needs to be allowed)."))
                )
    .subcommand(
        Command::new("info")
                .about("Show info related to node")
                .arg(Arg::new("join_address")
                        .short('j')
                        .long("join-address")
                        .help("Show join address for every interface (Master node only)"))
                .arg(Arg::new("peers")
                        .short('p')
                        .long("peers")
                        .help("List of peers"))
                .arg(Arg::new("config")
                        .short('c')
                        .long("config")
                        .help("Show current node configuration"))
                )
    .subcommand(Command::new("edit")
                .about("Edit master node settings")
                .arg(Arg::new("timeout")
                    .short('t')
                    .long("timeout")
                    .takes_value(true)
                    .help("General connection timeout in seconds (Default: 60)"))
                .arg(Arg::new("client_port")
                    .short('c')
                    .long("client-port")
                    .takes_value(true)
                    .help("Port where to listen for incoming client requests (Default: 5900)"))
                .arg(Arg::new("local_peer_port")
                    .short('l')
                    .long("local-peer-port")
                    .takes_value(true)
                    .help("Local port for nodes communication (Default: 5901)"))
                .arg(Arg::new("node_type")
                    .short('n')
                    .long("node-type")
                    .help("Local node type")
                    .takes_value(true)
                    .possible_values(["master", "normal"]))
                )
    .subcommand(Command::new("new-network")
                .about("Create a new network")
                .arg(Arg::new("root")
                    .short('r')
                    .long("root")
                    .takes_value(true)
                    .required(true)
                    .help("Specify root public key for network."))
                )
    .subcommand(Command::new("join-network")
                .about("Join existing network")
                .arg(Arg::new("peer")
                    .short('p')
                    .long("peer")
                    .takes_value(true)
                    .required(true)
                    .help("Specify master node peer to join."))
                )
    .get_matches();

    let mut settings: Settings;
    let mut daemon_conf = None;
    // check if our data dir exist, if not create it
    let mut conf_dir_clone = NYUMT_DIR.clone();
    if !conf_dir_clone.exists() {
        match fs::create_dir(&conf_dir_clone) {
            Ok(_)  => (),
            Err(e) => {
                error!("Error creating nyumt with path {}: {}", conf_dir_clone.to_str().unwrap(), e);
                return Err(Box::new( std::io::Error::new( std::io::ErrorKind::Other, e ) ));
            }
        }
    }

    conf_dir_clone.push("node_settings.json");
    let settings_file = match conf_dir_clone.to_str() {
            Some(v) => v,
            None    => {
                error!("Error reading from config file node_settings.json");
                return Err(Box::new( std::io::Error::new( std::io::ErrorKind::Other, "Error reading from config file" ) ));
            }
    };
    if !conf_dir_clone.exists() {
        info!("Config file not found, creating config file with default settings");
        settings = Settings {
            master_node: false,
            timeout: 60,
            client_port: 5901,
            local_peer_port: 5902,
            peers: Vec::new()
        };
        std::fs::write(settings_file, serde_json::to_string(&settings)?.as_bytes())?;
    } else {
        let file = std::fs::read(settings_file)?;
        settings = match serde_json::from_slice(&file) {
            Ok(v) => v,
            Err(e) => {
                error!("Error parsing config file node_settings.json: {}", e);
                return Err(Box::new( std::io::Error::new( std::io::ErrorKind::Other, e ) ));
            }
        };
        info!("Config file loaded: {:?}", settings);
    }

    let mut keys_file = NYUMT_DIR.clone();
    keys_file.push("keys.conf");

    let keypair;
    if !keys_file.exists() {
        keypair = create_keys().await?;
    } else {
        let keys = get_keypair().await?;
        info!("Keypair with public key {} loaded from key.conf", keys);
        keypair = keys;
    }

    if let Some(args) = glob_args.subcommand_matches("edit") {
        let mut update = false;
        if args.is_present("timeout") || args.is_present("client_port") || args.is_present("local_peer_port") || args.is_present("node_type") {
            settings = Settings {
                master_node: if args.is_present("node_type") {
                    let val: String = args.value_of_t("node_type")?;
                    if val == "master" {
                        true
                    } else {
                        false
                    }
                } else { settings.master_node },
                timeout: if args.is_present("timeout") { args.value_of_t("timeout")? } else { settings.timeout },
                client_port: if args.is_present("client_port") { args.value_of_t("client_port")? } else { settings.client_port },
                local_peer_port: if args.is_present("local_peer_port") { args.value_of_t("local_peer_port")? } else { settings.local_peer_port },
                peers: settings.peers
            };
            update = true;
        }

        if update {
            std::fs::write(settings_file, serde_json::to_string(&settings)?.as_bytes())?;
        }
    } else if let Some(args) = glob_args.subcommand_matches("info") {
        info::handle(&settings, args, keypair.public()).await;
    } else if let Some(args) = glob_args.subcommand_matches("daemon") {
        daemon_conf = Some(DaemonSettings {
            auto_discovery: if args.is_present("auto_discovery") { true } else { false }
        });
    } else if let Some(args) = glob_args.subcommand_matches("new-network") {
        if !settings.master_node {
            warn!("Can't create a new network with a normal node");
            std::process::exit(1);
        }
        let mut db_file = NYUMT_DIR.clone();
        db_file.push("storage.db");
        let overwrite;
        if db_file.exists() {
            print!("Node is already in a network. Do you want to overwrite database? (y)es, (n)o: ");
            std::io::stdout().lock().flush()?;
            let mut str = String::new();
            loop {
                std::io::stdin().read_line(&mut str)?;
                if str.trim() == "y" {
                    overwrite = true;
                    fs::remove_file(&db_file)?;
                    break;
                } else if str.trim() == "n" {
                    overwrite = false;
                    break;
                }
                str.clear();
                print!("(y)es, (n)o: ");
                std::io::stdout().lock().flush()?;
            }
        } else {
            overwrite = true;
        }
        if overwrite {
            let pk: String = args.value_of_t("root")?;
            let pk = match PublicKey::parse_str(pk.as_str()) {
                Ok(v) => v,
                Err(e) => {
                    error!("Error parsing public key: {}", pk);
                    return Err(e);
                }
            };
            let db = match database::Database::new(*NYUMT_DIR_STR, settings.master_node).await {
                Ok(v) => v,
                Err(e) => return Err(e)
            };
            master::first::create(&db, &keypair, &pk)?;
        }
    } else if let Some(args) = glob_args.subcommand_matches("join-network") {
        if args.is_present("peer") {
            let peer: String = args.value_of_t("peer")?;
            let parsed_peer  = info::parse_peer(&peer).await?;
            settings = Settings {
                master_node: settings.master_node,
                timeout: settings.timeout,
                client_port: settings.client_port,
                local_peer_port: settings.local_peer_port,
                peers: vec![parsed_peer]
            };
            std::fs::write(settings_file, serde_json::to_string(&settings)?.as_bytes())?;
            info!("Added {} with public key {} to be joined", settings.peers[0].addr, settings.peers[0].publickey);
        }
    }
    Ok((settings, daemon_conf, keypair))
}

async fn create_keys() -> Result<LocalPeerKey, Box<dyn Error>> {
    let keys = generate_keypair();
    match save_keypair(&keys).await {
        Ok(_) => (),
        Err(e) => {
            error!("Error saving keypair: {}", e);
            return Err(e);
        }
    }
    info!("Keypair with public key {} generated", keys.public());
    Ok(keys)
}

pub async fn start(stop: Sender<Stop>) -> Result<(), Box<dyn Error + Sync + Send>> {
    let (mut settings, daemon_conf, mut keys) = match arg_parse().await {
        Ok(v) => v,
        Err(_) => return Ok(())
    };

    if daemon_conf.is_none() {
        return Ok(());
    }

    let (tx, mut rx): (mpsc::Sender<Request>, mpsc::Receiver<Request>) = mpsc::channel(100);
    let db = database::Database::new(*NYUMT_DIR_STR, settings.master_node).await?;

    if settings.master_node {
        if database::block::get_last_block(&db).await?.is_none() && settings.peers.is_empty() {
            warn!("You must either create a new network or join one");
            return Ok(());
        }

        let _daemon_conf  = daemon_conf.unwrap();
        let shared_db     = db.clone();
        let tx_clone      = tx.clone();
        let mut keys_cl   = keys.clone();
        let stop_clone    = stop.clone();
        let mut set_clone = settings.clone();
        tokio::spawn(async move {
            match master::serve(&mut set_clone, &_daemon_conf, &mut rx, tx_clone, shared_db, &mut keys_cl, stop_clone).await {
                Ok(_)  => (),
                Err(e) => error!("Error occured in master node listener: {}", e)
            }
        });

        match master_serve::serve(&mut settings, db, &mut keys, stop, tx).await {
            Ok(_)  => (),
            Err(e) => error!("Error occured in client listener: {}", e)
        }
    } else {
        let ret = crate::database::node::node_master_node_count(&db).await;
        if (ret.is_err() || ret.is_ok() && ret.unwrap() == 0) && settings.peers.is_empty() {
            warn!("Normal node must have a network to join");
            return Ok(());
        }

        match node::serve(&mut settings, db, &mut keys, stop).await {
            Ok(_)  => (),
            Err(e) => error!("Error occured in node listener: {}", e)
        }
    }

    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let filter = match tracing_subscriber::EnvFilter::try_from_env("DEBUGGER") {
        Ok(v)  => v.add_directive("sqlx::query=debug".parse()?)
            .add_directive("netlink_proto=info".parse()?)
            .add_directive("debug".parse()?),
        Err(_) => tracing_subscriber::EnvFilter::from_default_env().add_directive("sqlx::query=off".parse()?)
            .add_directive("nyumt=info".parse()?)
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_thread_names(true)
        .init();

    let mut runtime;
    loop {
        runtime = Runtime::new().expect("Failed to start Runtime");

        let (tx, mut rx): (Sender<Stop>, Receiver<Stop>) = mpsc::channel(5);

        runtime.spawn({
            start(tx)
        });
        match futures::executor::block_on(rx.recv()) {
            Some(v) => match v {
                Stop::Panic => {
                    panic!("Fatal error occured!");
                },
                Stop::SoftRestart => {
                    runtime.shutdown_timeout(Duration::from_secs(5));
                    continue
                },
            },
            None => break
        }
    }
    Ok(())
}
