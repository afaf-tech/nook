#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! The DataCraft desktop app.

use datacraft_ui::Options;

fn usage() -> String {
    "datacraft — database administration for SQLite, PostgreSQL and MySQL

  datacraft                                  start with your saved connections
  datacraft --demo                           start with a sample SQLite database
  datacraft --conn postgres://localhost/shop open a connection at launch
  datacraft --sql \"select * from users\"      start with this query
  datacraft --control 7981                   serve the JSON control channel
  datacraft --shot /tmp/shot.png             render one frame to a PNG and exit

The same commands are available from the shell (`datacraft-cli commands`) and over MCP
(`datacraft-cli mcp`)."
        .to_string()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", usage());
        return;
    }
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--demo" => options.demo = true,
            "--control" => {
                index += 1;
                match args.get(index).and_then(|p| p.parse::<u16>().ok()) {
                    Some(port) => options.control_port = Some(port),
                    None => {
                        eprintln!("--control needs a port number");
                        std::process::exit(2);
                    }
                }
            }
            "--shot" => {
                index += 1;
                match args.get(index) {
                    Some(path) => options.shot = Some(path.clone()),
                    None => {
                        eprintln!("--shot needs a file path");
                        std::process::exit(2);
                    }
                }
            }
            "--conn" => {
                index += 1;
                match args.get(index) {
                    Some(uri) => options.connect.push(uri.clone()),
                    None => {
                        eprintln!("--conn needs a connection string");
                        std::process::exit(2);
                    }
                }
            }
            "--sql" => {
                index += 1;
                match args.get(index) {
                    Some(sql) => options.sql = Some(sql.clone()),
                    None => {
                        eprintln!("--sql needs a statement");
                        std::process::exit(2);
                    }
                }
            }
            other => {
                eprintln!("unknown argument '{other}'\n\n{}", usage());
                std::process::exit(2);
            }
        }
        index += 1;
    }
    if let Err(e) = datacraft_ui::run(options) {
        eprintln!("datacraft: {e}");
        std::process::exit(1);
    }
}
