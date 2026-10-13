#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! The Nook desktop app.

use nook_ui::Options;

fn usage() -> String {
    "nook — database administration for SQLite, PostgreSQL and MySQL

  nook                                  start with your saved connections
  nook --demo                           start with a sample SQLite database
  nook --conn postgres://localhost/shop open a connection at launch
  nook --sql \"select * from users\"      start with this query
  nook --control 7981                   serve the JSON control channel
  nook --shot /tmp/shot.png             render one frame to a PNG and exit
  nook --ssh-askpass <id>               print the connection's stored SSH password (ssh runs this)

The same commands are available from the shell (`nook-cli commands`) and over MCP
(`nook-cli mcp`)."
        .to_string()
}

/// The helper `ssh` execs when a tunnel needs the stored SSH password: print one line and exit. The
/// connection id arrives in argv, or in `NOOK_ASKPASS_CONN` because ssh's askpass protocol
/// passes only the prompt — the password itself is read here, in the helper, so it never reaches the
/// parent's arguments or environment.
fn askpass(id: Option<String>) -> i32 {
    let Some(id) = id else {
        eprintln!("--ssh-askpass needs a connection id");
        return 2;
    };
    match nook_engine::profiles::ssh_askpass_line(&id) {
        Ok(secret) => {
            println!("{secret}");
            0
        }
        Err(e) => {
            eprintln!("nook --ssh-askpass: {e}");
            1
        }
    }
}

/// Is this process `ssh` asking for the password, and for which connection? `ssh` execs `SSH_ASKPASS`
/// with the prompt and nothing else, so the invocation is one non-flag argument plus the id in
/// `NOOK_ASKPASS_CONN`; `None` means an ordinary launch, which a stray environment cannot turn
/// into one.
fn askpass_invocation(args: &[String], env: Option<String>) -> Option<Option<String>> {
    let mut rest = args.iter();
    if let Some(flag) = rest.find(|a| *a == "--ssh-askpass") {
        let _ = flag;
        return Some(rest.next().cloned().or(env));
    }
    match args {
        [prompt] if !prompt.starts_with('-') && env.is_some() => Some(env),
        _ => None,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", usage());
        return;
    }
    // `ssh` runs the app this way — with the prompt as its only argument and the id in the
    // environment — so it is answered before any option is parsed, and it prints one line and exits
    // instead of starting up.
    if let Some(id) = askpass_invocation(&args, std::env::var(nook_engine::ASKPASS_CONN_ENV).ok()) {
        std::process::exit(askpass(id));
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
    if let Err(e) = nook_ui::run(options) {
        eprintln!("nook: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Which invocations are `ssh` asking for a password. `ssh` execs `SSH_ASKPASS` with the prompt
    /// as its only argument, so the id can only come from `NOOK_ASKPASS_CONN`; the flag form is
    /// the manual one, and a stray environment must not turn an ordinary launch into a helper.
    #[test]
    fn ssh_askpass_is_recognised_by_its_own_invocation() {
        let prompt = vec!["ops@jump.example.com's password:".to_string()];
        assert_eq!(
            askpass_invocation(&prompt, Some("shop".into())),
            Some(Some("shop".into())),
            "ssh's own invocation"
        );
        assert_eq!(
            askpass_invocation(&["--ssh-askpass".into(), "shop".into()], None),
            Some(Some("shop".into())),
            "the flag names the connection"
        );
        assert_eq!(
            askpass_invocation(&["--ssh-askpass".into()], Some("shop".into())),
            Some(Some("shop".into())),
            "the flag falls back to the environment"
        );
        assert_eq!(
            askpass_invocation(&["--ssh-askpass".into()], None),
            Some(None),
            "a missing id is an error the helper reports"
        );
        assert_eq!(askpass_invocation(&prompt, None), None, "no id anywhere");
        assert_eq!(
            askpass_invocation(&["--demo".into()], Some("shop".into())),
            None,
            "a stray environment does not hijack a normal launch"
        );
        assert_eq!(askpass_invocation(&[], Some("shop".into())), None);
        assert_eq!(
            askpass_invocation(&["a".into(), "b".into()], Some("shop".into())),
            None,
            "ssh passes exactly one argument"
        );
    }
}
