use std::{error::Error, sync::Arc};
use tokio::sync::{RwLock, oneshot};
use clap::{Command, Arg, ArgMatches};
use nyumt_client_core::{storage::{Storage, Session, self}, deadpass::SecretVal, client::Client};
use rustyline::{Editor, error::ReadlineError};
use shellwords::split;
use futures::{StreamExt, TryFutureExt};
use borsh::BorshDeserialize;

use nyumt_client_core::{order, db::{ClientDatabase, Database}, stats};
use nyumt_proto::{database::{auth, node, template}, types::{Permissions, RequestStatus}, keys::PublicKey, api::RequestResp};

use crate::plot;

pub async fn manage(mut db: Storage) -> Result<(), Box<dyn Error>> {
    let db = Arc::new(RwLock::new(db));

    let mut rl = Editor::<()>::new();
    'READ: loop {
        let readline = rl.readline(">> ");
        match readline {
            Ok(line) => {
                let mut command = String::from("session ");
                command.push_str(line.as_str());
                rl.add_history_entry(line.as_str());
                let args = match Command::new("Session")
                    .help_expected(true)
                    .subcommand(Command::new("list")
                                .about("List sessions")
                                )
                     .subcommand(Command::new("open")
                                .about("Open session.")
                                .arg(Arg::new("name")
                                    .required(true)
                                    .takes_value(true)
                                    .help("Session name"))
)
                    .subcommand(Command::new("new")
                                .about("Create new session.")
                                .arg(Arg::new("host")
                                    .required(true)
                                    .takes_value(true)
                                    .help("Nyumt master node server address [ip|domain]:port"))
                                .arg(Arg::new("name")
                                    .required(true)
                                    .takes_value(true)
                                    .help("Session name"))
                                ).try_get_matches_from(match split(command.as_str()) {
                    Ok(v) => v,
                    Err(_) => {
                        println!("Error parsing command");
                        continue;
                    }
                }) {
                    Ok(v) => v,
                    Err(e) => {
                        e.print().ok();
                        continue
                    }
                };

                if let Some(_) = args.subcommand_matches("list") {
                    println!(
                        "{0: <10} | {1: <10} | {2: <40} | {3: <40}",
                        "id", "name", "host", "user publickey"
                    );
                    let mut count = 0;
                    for session in &db.read().await.get().session.0 {
                        println!(
                            "{0: <10} | {1: <10} | {2: <40} | {3: <40}",
                            count, session.name.as_str(), session.host.as_str(), session.keys
                        );
                        count += 1;
                    }
                } else if let Some(sub_args) = args.subcommand_matches("new") {
                    let host: String = sub_args.value_of_t("host")?;
                    let name: String = sub_args.value_of_t("name")?;
                    for session in &db.read().await.get().session.0 {
                        if session.name.as_str() == name {
                            println!("Session with same name {} exists", name);
                            continue 'READ;
                        }
                    }
                    let keys = storage::KeyPair::generate();
                    db.write().await.get_mut().session.0.push(Session {
                        name: SecretVal(name),
                        host: SecretVal(host),
                        keys,
                    });
                    db.write().await.save()?;
                } else if let Some(sub_args) = args.subcommand_matches("open") {
                    let name: String  = sub_args.value_of_t("name")?;
                    let mut sess_id: Option<(usize, String)> = None;
                    let mut count = 0;
                    for session in &db.read().await.get().session.0 {
                        if session.name.as_str() == name {
                            sess_id = Some((count, session.host.to_owned()));
                            break;
                        }
                        count += 1;
                    }
                    match sess_id {
                        Some(v) => connect(db.clone(), &name, v.0, &v.1).await,
                        None => {
                            println!("Session with name {} not found", name);
                            continue;
                        }
                    }
                }
            },
            Err(ReadlineError::Interrupted |  ReadlineError::Eof) => {
                return Ok(());
            },
            Err(err) => {
                println!("Error reading line: {}", err);
                return Ok(());
            }
        }
    }
}

async fn connect(storage: Arc<RwLock<Storage>>, name: &str, session_id: usize, host: &str) {
    let db = match ClientDatabase::new(name).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error opening database: {}", e);
            return;
        }
    };
    let (client, mut disconnected, close) = match Client::new(storage.clone(), session_id, db.clone()).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error connecting to {}: {}", host, e);
            return;
        }
    };

    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(futures::future::select(Box::pin(session(storage, name.to_owned(), session_id, host.to_owned(), client, db, close)), receiver.map_err(drop)));
    tokio::select! {
        _ = &mut disconnected => {
            println!("Master node {} closed connection", host);
            sender.send(()).ok();

            return
        },
    };
}

async fn session(storage: Arc<RwLock<Storage>>, name: String, session_id: usize, host: String, mut client: Client, db: Database, close: oneshot::Sender<()>) {
    tokio::task::spawn_blocking(move || {
        let mut rl = Editor::<()>::new();
        let tag = format!("({}@{})>> ", name, host);
        loop {
            let readline = rl.readline(&tag);
            match readline {
                Ok(line) => {
                    let mut command = String::from("session ");
                    command.push_str(line.as_str());
                    rl.add_history_entry(line.as_str());
                    let args = match Command::new("Nyumt control panel")
                        .help_expected(true)
                        .subcommand(Command::new("block")
                                    .about("List blocks")
                                    )
                         .subcommand(Command::new("user")
                                    .about("User management")
                                    .subcommand(
                                        Command::new("list")
                                            .about("List all users")
                                    )
                                    .subcommand(
                                        Command::new("add")
                                            .about("Add new user")
                                            .arg(Arg::new("permission")
                                                 .short('p')
                                                 .long("permission")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .possible_values(["root", "monitor"])
                                                 .multiple_values(true)
                                                 .help("User permission"))
                                            .arg(Arg::new("name")
                                                 .short('n')
                                                 .long("name")
                                                .required(true)
                                                .takes_value(true)
                                                .help("User name"))
                                            .arg(Arg::new("publickey")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("User public key"))
                                    )
                                    .subcommand(
                                        Command::new("remove")
                                            .about("Remove user")
                                            .arg(Arg::new("publickey")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("User public key"))
                                    )
                                    .subcommand(
                                        Command::new("edit")
                                            .about("Edit user")
                                            .arg(Arg::new("permission")
                                                 .short('p')
                                                 .long("permission")
                                                 .possible_values(["root", "monitor"])
                                                 .multiple_values(true)
                                                 .help("User permission"))
                                            .arg(Arg::new("name")
                                                .short('n')
                                                .long("name")
                                                .required(true)
                                                .takes_value(true)
                                                .help("User name"))
                                            .arg(Arg::new("publickey")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("User public key"))
                                    )
                         )
                         .subcommand(Command::new("node")
                                    .about("Node management")
                                    .subcommand(
                                        Command::new("list")
                                            .about("List all nodes")
                                    )
                                    .subcommand(
                                        Command::new("add")
                                            .about("Add new node")
                                            .arg(Arg::new("master")
                                                 .short('m')
                                                 .long("master")
                                                 .help("Node is a master node"))
                                            .arg(Arg::new("name")
                                                 .short('n')
                                                 .long("name")
                                                .required(true)
                                                .takes_value(true)
                                                .help("Node name"))
                                            .arg(Arg::new("publickey")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("Node public key"))
                                    )
                                    .subcommand(
                                        Command::new("remove")
                                            .about("Remove node")
                                            .arg(Arg::new("publickey")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("Node public key"))
                                    )
                                    .subcommand(
                                        Command::new("template")
                                            .about("Change node template")
                                            .arg(Arg::new("template")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("Template name"))
                                            .arg(Arg::new("publickey")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("Node public key"))
                                    )
                                    .subcommand(
                                        Command::new("edit")
                                            .about("Edit node")
                                            .arg(Arg::new("name")
                                                .short('n')
                                                .long("name")
                                                .required(true)
                                                .takes_value(true)
                                                .help("Node name"))
                                            .arg(Arg::new("publickey")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("Node public key"))
                                    )
                                    .subcommand(
                                        Command::new("get")
                                            .about("Get stats from node")
                                            .arg(Arg::new("OID")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("Object identifier"))
                                            .arg(Arg::new("publickey")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("Node public key"))
                                    )
                                    .subcommand(
                                        Command::new("plot")
                                            .about("Plot graph of an object with type Volatile")
                                            .arg(Arg::new("OID")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("Object identifier"))
                                            .arg(Arg::new("publickey")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("Node public key"))
                                    )
                        )
                        .subcommand(Command::new("info-struct")
                                    .about("Show management information fields available")
                        )
                        .subcommand(Command::new("template")
                                    .about("Template management")
                                    .subcommand(
                                        Command::new("list")
                                            .about("List all templates")
                                    )
                                    .subcommand(
                                        Command::new("generate")
                                            .about("Generate new json template config file")
                                            .arg(Arg::new("output")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("output"))
                                    )
                                    .subcommand(
                                        Command::new("show")
                                            .about("Show template")
                                            .arg(Arg::new("name")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("Template name"))
                                    )
                                    .subcommand(
                                        Command::new("set")
                                            .about("Set template")
                                            .arg(Arg::new("name")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("Template"))
                                            .arg(Arg::new("config_file")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("Template config file"))
                                    )
                                    .subcommand(
                                        Command::new("remove")
                                            .about("Remove template")
                                            .arg(Arg::new("name")
                                                 .required(true)
                                                 .takes_value(true)
                                                 .help("Template name"))
                                    )
                        ).try_get_matches_from(match split(command.as_str()) {
                        Ok(v) => v,
                        Err(_) => {
                            println!("Error parsing command");
                            continue;
                        }
                    }) {
                        Ok(v) => v,
                        Err(e) => {
                            e.print().ok();
                            continue
                        }
                    };

                    if let Some(sub_args) = args.subcommand_matches("user") {
                        match futures::executor::block_on(user(&mut client, storage.clone(), session_id, sub_args, &db)) {
                            Err(e) => println!("Error: {}", e),
                            _ => ()
                        }
                    } else if let Some(sub_args) = args.subcommand_matches("node") {
                        match futures::executor::block_on(node(&mut client, storage.clone(), session_id, sub_args, &db)) {
                            Err(e) => println!("Error: {}", e),
                            _ => ()
                        }
                    } else if let Some(_) = args.subcommand_matches("info-struct") {
                        futures::executor::block_on(db_struct());
                    } else if let Some(sub_args) = args.subcommand_matches("template") {
                        match futures::executor::block_on(template(&mut client, storage.clone(), session_id, sub_args, &db)) {
                            Err(e) => println!("Error: {}", e),
                            _ => ()
                        }
                    }
                },
                Err(ReadlineError::Interrupted |  ReadlineError::Eof) => {
                    close.send(()).ok();
                    return;
                },
                Err(err) => {
                    eprintln!("Error reading line: {}", err);
                }
            }
        }
    }).await.ok();
}

async fn user(client: &mut Client, storage: Arc<RwLock<Storage>>, session_id: usize, args: &ArgMatches, db: &Database) -> Result<(), Box<dyn Error>> {
    if let Some(_) = args.subcommand_matches("list") {
        let mut user_reader = auth::get_users(db).await;
        println!("{0: <10} | {1: <20} | {2: <30} | {3}",
                 "Id", "Name", "Permissions", "Public key"
                 );
        loop {
            let user = user_reader.next().await;
            match user {
                Some(v) => match v {
                    Ok(v) => {
                        if v.active {
                            println!(
                                "{0: <10} | {1: <20} | {2: <30} | {3}",
                                v.id, v.name, Permissions::try_from_slice(&v.permissions)?, PublicKey::try_from_slice(&v.public_key)?
                            );
                        }
                    },
                    Err(e) => {
                        eprintln!("Error getting user from database: {}", e);
                        break;
                    }
                },
                None => break
            }
        };
    } else if let Some(sub_args) = args.subcommand_matches("add") {
        let name: String            = sub_args.value_of_t("name")?;
        let permission: Vec<String> = sub_args.values_of_t("permission")?;
        let publickey: String       = sub_args.value_of_t("publickey")?;

        match order::add_user(storage, session_id, &publickey, &name, &permission).await {
            Ok(v) => {
                match client.execute(v).await {
                    Ok(v) => println!("{:?}", v),
                    Err(e) => println!("{}", e)
                }
            },
            Err(e) => eprintln!("Error adding user: {}", e)
        }
    } else if let Some(sub_args) = args.subcommand_matches("edit") {
        let name: String            = sub_args.value_of_t("name")?;
        let permission: Vec<String> = sub_args.values_of_t("permission")?;
        let publickey: String       = sub_args.value_of_t("publickey")?;

        match order::update_user(storage, session_id, &publickey, &name, &permission).await {
            Ok(v) => {
                match client.execute(v).await {
                    Ok(v) => println!("{:?}", v),
                    Err(e) => println!("{}", e)
                }
            },
            Err(e) => eprintln!("Error adding user: {}", e)
        }
    } else if let Some(sub_args) = args.subcommand_matches("remove") {
        let publickey: String       = sub_args.value_of_t("publickey")?;

        match order::remove_user(storage, session_id, &publickey).await {
            Ok(v) => {
                match client.execute(v).await {
                    Ok(v) => println!("{:?}", v),
                    Err(e) => println!("{}", e)
                }
            },
            Err(e) => eprintln!("Error adding user: {}", e)
        }
    }
    Ok(())
}

async fn node(client: &mut Client, storage: Arc<RwLock<Storage>>, session_id: usize, args: &ArgMatches, db: &Database) -> Result<(), Box<dyn Error>> {
    if let Some(_) = args.subcommand_matches("list") {
        let mut node_reader = node::get_nodes(db).await;
        println!("{0: <10} | {1: <20} | {2: <15} | {3: <15} | {4}",
                 "Id", "Name", "Type", "Template", "Public key"
                 );
        loop {
            let node = node_reader.next().await;
            match node {
                Some(v) => match v {
                    Ok(v) => {
                        println!(
                            "{0: <10} | {1: <20} | {2: <15} | {3: <15} | {4}",
                            v.id, v.name, if v.master_node { "Master" } else { "Normal" }, v.conf, PublicKey::try_from_slice(&v.public_key)?
                        );
                    },
                    Err(e) => {
                        eprintln!("Error getting node from database: {}", e);
                        break;
                    }
                },
                None => break
            }
        };
    } else if let Some(sub_args) = args.subcommand_matches("add") {
        let name: String            = sub_args.value_of_t("name")?;
        let publickey: String       = sub_args.value_of_t("publickey")?;

        match order::add_node(storage, session_id, &publickey, &name, sub_args.is_present("master")).await {
            Ok(v) => {
                match client.execute(v).await {
                    Ok(v) => println!("{:?}", v),
                    Err(e) => println!("{}", e)
                }
            },
            Err(e) => eprintln!("Error adding user: {}", e)
        }
    } else if let Some(sub_args) = args.subcommand_matches("edit") {
        let name: String            = sub_args.value_of_t("name")?;
        let publickey: String       = sub_args.value_of_t("publickey")?;

        match order::update_node(&db, storage, session_id, &publickey, &name).await {
            Ok(v) => {
                match client.execute(v).await {
                    Ok(v) => println!("{:?}", v),
                    Err(e) => println!("{}", e)
                }
            },
            Err(e) => eprintln!("Error editing user: {}", e)
        }
    } else if let Some(sub_args) = args.subcommand_matches("remove") {
        let publickey: String       = sub_args.value_of_t("publickey")?;

        match order::remove_node(&db, storage, session_id, &publickey).await {
            Ok(v) => {
                match client.execute(v).await {
                    Ok(v) => println!("{:?}", v),
                    Err(e) => println!("{}", e)
                }
            },
            Err(e) => eprintln!("Error removing user: {}", e)
        }
    } else if let Some(sub_args) = args.subcommand_matches("template") {
        let publickey: String       = sub_args.value_of_t("publickey")?;
        let name: String            = sub_args.value_of_t("template")?;

        match order::change_node_config(storage, session_id, &publickey, &name).await {
            Ok(v) => {
                match client.execute(v).await {
                    Ok(v) => println!("{:?}", v),
                    Err(e) => println!("{}", e)
                }
            },
            Err(e) => eprintln!("Error changing template for node: {}", e)
        }
    } else if let Some(sub_args) = args.subcommand_matches("get") {
        let publickey: String = sub_args.value_of_t("publickey")?;
        let oid: String       = sub_args.value_of_t("OID")?;

        let oid = stats::parse_oid(&oid).await?;

        let pk = PublicKey::parse_str(&publickey)?;
        match client.get_stats(pk, oid).await {
            Ok(v) => {
                match v {
                    RequestResp::RequestStatus(v) => {
                        match v {
                            RequestStatus::GetStats(rep, _) => {
                                let stats = stats::interpret_stats(&rep).await;
                                for val in stats {
                                    println!("{} = {:?}", val.0, val.1);
                                }
                            },
                            e @ _ => println!("Error occured: {:?}", e)
                        }
                    },
                    RequestResp::RequestError(e) => println!("Error occured: {:?}", e)
                }
            },
            Err(e) => println!("{}", e)
        }
    } else if let Some(sub_args) = args.subcommand_matches("plot") {
        let publickey: String  = sub_args.value_of_t("publickey")?;
        let oid_string: String = sub_args.value_of_t("OID")?;

        let oid = stats::parse_oid(&oid_string).await?;

        let pk = PublicKey::parse_str(&publickey)?;

        let mut r = stats::plot_data(&db, &pk, oid, 0).await?;
        
        let mut plot_data = Vec::new();
        loop {
            let v = r.next().await;
            match v {
                Some(Ok(v)) => {
                    plot_data.push(v);
                },
                _ => break
            }
        };
        tokio::task::spawn_blocking(move || {
            plot::show(&oid_string, plot_data).ok();
        }).await?;
    }
    Ok(())
}

async fn template(client: &mut Client, storage: Arc<RwLock<Storage>>, session_id: usize, args: &ArgMatches, db: &Database) -> Result<(), Box<dyn Error>> {
    if let Some(sub_args) = args.subcommand_matches("generate") {
        let output: String = sub_args.value_of_t("output")?;
        
        let mut config = nyumt_proto::config::GlobalConfig::default();
        let stat_conf  = nyumt_proto::stats::DbStruct::generate();
        
        config.node.db_struct = stat_conf;

        tokio::fs::write(&output, serde_json::to_string_pretty(&config)?.as_bytes()).await?;

        println!("Template config file generated to {}/{}", std::env::current_dir()?.to_str().unwrap(), output);
    } else if let Some(_) = args.subcommand_matches("list") {
        let mut node_reader = template::get_template_list(db).await;
        println!("{0: <30} | {1}",
                 "Name", "Total nodes using template"
                 );
        loop {
            let node = node_reader.next().await;
            match node {
                Some(v) => match v {
                    Ok(v) => {
                        println!("{0: <30} | {1}",
                            v.name, v.used_by
                        );
                    },
                    Err(e) => {
                        eprintln!("Error getting template info from database: {}", e);
                        break;
                    }
                },
                None => break
            }
        };
    } else if let Some(sub_args) = args.subcommand_matches("show") {
        let name: String = sub_args.value_of_t("name")?;

        let config = template::get_template(db, &name).await;

        match config {
            Ok(Some(v)) => {
                let master = nyumt_proto::config::MasterConfig::try_from_slice(&v.master_conf)?;
                let node   = nyumt_proto::config::NodeConfig::try_from_slice(&v.node_conf)?;
                let config = nyumt_proto::config::GlobalConfig { master, node };

                println!("{}", serde_json::to_string_pretty(&config)?);
            },
            _ => println!("Template with name {} not found", name)
        }
    } else if let Some(sub_args) = args.subcommand_matches("set") {
        let conf_file: String = sub_args.value_of_t("config_file")?;
        let name: String      = sub_args.value_of_t("name")?;

        let config: nyumt_proto::config::GlobalConfig;
        {
            let file = tokio::fs::read(conf_file).await?;
            config   = serde_json::from_slice(&file)?;
        }

        match order::set_config(storage, session_id, &name, &config.master, &config.node).await {
            Ok(v) => {
                match client.execute(v).await {
                    Ok(v) => println!("{:?}", v),
                    Err(e) => println!("{}", e)
                }
            },
            Err(e) => eprintln!("Error setting template: {}", e)
        }
    } else if let Some(sub_args) = args.subcommand_matches("remove") {
        let name: String      = sub_args.value_of_t("name")?;

        match order::remove_config(storage, session_id, &name).await {
            Ok(v) => {
                match client.execute(v).await {
                    Ok(v) => println!("{:?}", v),
                    Err(e) => println!("{}", e)
                }
            },
            Err(e) => eprintln!("Error removing template: {}", e)
        }
    }

    Ok(())
}

async fn db_struct() {
    for module in &*stats::DB_STRUCT {
        for field in &module.1 {
            println!("0.{}.{}: {:?}, {:?}", module.0, field.0, field.1, field.2);
        }
    }
}
