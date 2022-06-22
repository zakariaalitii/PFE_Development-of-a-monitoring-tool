mod session;
mod plot;

use clap::{Command, Arg};
use std::error::Error;
use std::io::Write;
use nyumt_client_core::storage::Storage;

fn parse_args() -> Result<Storage, Box<dyn Error>> {
    let glob_args = Command::new("Nyumt client")
    .version("1.0")
    .author("zesty")
    .subcommand(Command::new("new")
                .about("Create new session database")
                .arg(Arg::new("db_file")
                    .help("Database file")
                    .required(true)
                    .takes_value(true))
                )
    .subcommand(Command::new("open")
                .about("Open existing session database")
                .arg(Arg::new("db_file")
                    .help("Database file")
                    .required(true)
                    .takes_value(true))
                )
    .get_matches();

    nyumt_client_core::init();

    let mut db;
    if let Some(sub_args) = glob_args.subcommand_matches("new") {
        let db_path: String = sub_args.value_of_t("db_file")?;
        let pass  = rpassword::prompt_password_stdout("Password: ")?;
        let pass2 = rpassword::prompt_password_stdout("Verify password: ")?;
        if pass != pass2 {
            println!("Different passwords entered");
            std::process::exit(1);
        }
        print!("Keyfile path (leave empty if you don't want any): ");
        std::io::stdout().lock().flush()?;
        let mut keyfile_path = String::new();
        std::io::stdin().read_line(&mut keyfile_path)?;
        println!("Creating database...");
        if keyfile_path.trim() != "" {
            db = Storage::new(&db_path, pass.as_bytes(), Some(keyfile_path.trim()), None)?;
        } else {
            db = Storage::new(&db_path, pass.as_bytes(), None, None)?;
        }
        db.save()?;
    } else if let Some(sub_args) = glob_args.subcommand_matches("open") {
        let db_path: String = sub_args.value_of_t("db_file")?;
        let pass  = rpassword::prompt_password_stdout("Password: ")?;
        print!("Keyfile path (leave empty if none used): ");
        std::io::stdout().lock().flush()?;
        let mut keyfile_path = String::new();
        std::io::stdin().read_line(&mut keyfile_path)?;
        println!("Opening database...");
        if keyfile_path.trim() != "" {
            db = Storage::open(&db_path, pass.as_bytes(), Some(keyfile_path.trim()))?;
        } else {
            db = Storage::open(&db_path, pass.as_bytes(), None)?;
        }
    } else {
        panic!("This should not happen, unknewn subcommand");
    }

    Ok(db)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let db = parse_args()?;
    session::manage(db).await;
    Ok(())
}
