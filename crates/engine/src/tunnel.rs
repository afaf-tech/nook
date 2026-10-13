//! SSH tunnel: DataCraft shells out to the system `ssh`, so agent keys and `~/.ssh/config` just work.

use std::ffi::OsString;
use std::io::{ErrorKind, IsTerminal};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use datacraft_core::{ConnectionConfig, Error, Result};

/// The environment variable the askpass helper reads the connection id from. It is not a secret, and
/// it cannot be an argument: `ssh` execs `SSH_ASKPASS` with the prompt and nothing else.
pub const ASKPASS_CONN_ENV: &str = "DATACRAFT_ASKPASS_CONN";

const SSH_ASKPASS_ENV: &str = "SSH_ASKPASS";
const SSH_ASKPASS_REQUIRE_ENV: &str = "SSH_ASKPASS_REQUIRE";

/// Where the tunnel's SSH password comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshSecret {
    /// The login keychain: `ssh` reads it back through `--ssh-askpass`, so the secret is never in an
    /// argument, an environment variable or a file.
    Keychain(String),
    /// Nothing stored, and a terminal to ask on: `ssh` inherits the app's stdin and prompts there.
    /// A `ponytail:` that prompt is the terminal the app was started from, not a field of ours — an
    /// in-app prompt whose answer goes to the keychain for the length of the run is the upgrade.
    Prompt,
    /// Nothing stored and nowhere to ask (MCP reads its protocol from stdin): keys and the agent.
    None,
}

impl SshSecret {
    /// What a profile's tunnel gets: its keychain secret where one is stored, ssh's own prompt where
    /// there is a terminal for it, and nothing where there is neither.
    pub fn for_connection(id: &str) -> SshSecret {
        if let Ok(Some(_)) = crate::profiles::keychain_lookup(&crate::profiles::ssh_account(id)) {
            return SshSecret::Keychain(id.to_owned());
        }
        if std::io::stdin().is_terminal() {
            SshSecret::Prompt
        } else {
            SshSecret::None
        }
    }
}

/// What `ssh` is handed along with its own invocation. The secret is deliberately absent: the helper
/// reads the keychain itself, and a `Keychain` secret travels as the connection id.
#[derive(Debug, PartialEq, Eq)]
struct SpawnPlan {
    args: Vec<String>,
    env: Vec<(&'static str, OsString)>,
    inherit_stdin: bool,
}

fn spawn_plan(cfg: &ConnectionConfig, local_port: u16, secret: &SshSecret) -> SpawnPlan {
    // Only `None` means ssh may not ask at all; a keychain secret asks through the helper and a
    // terminal-free prompt is no prompt.
    let mut plan = SpawnPlan {
        args: cfg
            .ssh
            .ssh_args(&cfg.host, cfg.port, local_port, *secret != SshSecret::None),
        env: Vec::new(),
        inherit_stdin: false,
    };
    match secret {
        SshSecret::Keychain(id) => {
            plan.env = vec![
                (
                    SSH_ASKPASS_ENV,
                    std::env::current_exe().unwrap_or_default().into_os_string(),
                ),
                (SSH_ASKPASS_REQUIRE_ENV, OsString::from("force")),
                (ASKPASS_CONN_ENV, OsString::from(id)),
            ];
        }
        SshSecret::Prompt => plan.inherit_stdin = true,
        SshSecret::None => {}
    }
    plan
}

/// The hop the design's test row states (`jump.example.com:22 · local 15432`): which jump host the
/// tunnel used and the local port it actually bound, which `free_port()` chooses when the profile
/// asks for 0.
pub(crate) fn tunnel_facts(t: &Tunnel) -> serde_json::Value {
    serde_json::json!({
        "host": t.jump_host,
        "port": t.jump_port,
        "local_port": t.local_port,
    })
}

/// A running `ssh -N -L …` process. Dropping it closes the tunnel.
pub struct Tunnel {
    child: Child,
    pub local_port: u16,
    /// The jump host the tunnel runs through.
    pub jump_host: String,
    pub jump_port: u16,
    /// `host:port` of the database the tunnel forwards to.
    pub remote: String,
    pub command: String,
}

impl std::fmt::Debug for Tunnel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tunnel")
            .field("local_port", &self.local_port)
            .field("remote", &self.remote)
            .finish()
    }
}

impl Tunnel {
    /// Pick a free local port so two profiles can tunnel to the same server.
    pub fn free_port() -> Result<u16> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|e| Error::io(format!("no free local port: {e}")))?;
        let port = listener
            .local_addr()
            .map_err(|e| Error::io(format!("local addr: {e}")))?
            .port();
        drop(listener);
        Ok(port)
    }

    /// Start the tunnel for a profile whose `ssh.enabled` is set, and wait until the port answers.
    /// `secret` is where a password may come from — see `SshSecret`.
    pub fn start(cfg: &ConnectionConfig, secret: SshSecret) -> Result<Tunnel> {
        if !cfg.ssh.enabled {
            return Err(Error::config("this profile has no SSH tunnel enabled"));
        }
        if cfg.ssh.host.trim().is_empty() || cfg.ssh.user.trim().is_empty() {
            return Err(Error::config(
                "the SSH tunnel needs a jump host and a user name",
            ));
        }
        let local_port = if cfg.ssh.local_port == 0 {
            Tunnel::free_port()?
        } else {
            cfg.ssh.local_port
        };
        let plan = spawn_plan(cfg, local_port, &secret);
        let command = format!("ssh {}", plan.args.join(" "));
        let mut ssh = Command::new("ssh");
        ssh.args(&plan.args)
            .stdin(if plan.inherit_stdin {
                Stdio::inherit()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in plan.env {
            ssh.env(key, value);
        }
        let child = ssh
            .spawn()
            .map_err(|e| Error::connect(format!("could not start ssh: {e}")))?;
        let mut tunnel = Tunnel {
            child,
            local_port,
            jump_host: cfg.ssh.host.clone(),
            jump_port: cfg.ssh.port,
            remote: format!("{}:{}", cfg.host, cfg.port),
            command,
        };
        tunnel.wait_ready(Duration::from_secs(cfg.connect_timeout_s.clamp(5, 60)))?;
        Ok(tunnel)
    }

    fn wait_ready(&mut self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                let stderr = self
                    .child
                    .stderr
                    .take()
                    .map(|mut s| {
                        use std::io::Read;
                        let mut buf = String::new();
                        let _ = s.read_to_string(&mut buf);
                        buf
                    })
                    .unwrap_or_default();
                return Err(Error::connect(format!(
                    "ssh exited with {status}: {}",
                    stderr.trim()
                )));
            }
            match TcpStream::connect_timeout(
                &format!("127.0.0.1:{}", self.local_port)
                    .parse()
                    .map_err(|e| Error::internal(format!("bad local address: {e}")))?,
                Duration::from_millis(400),
            ) {
                Ok(_) => return Ok(()),
                Err(e) if e.kind() == ErrorKind::ConnectionRefused => {}
                Err(_) => {}
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                return Err(Error::connect(format!(
                    "ssh tunnel to {} did not open port {} in time",
                    self.remote, self.local_port
                )));
            }
            std::thread::sleep(Duration::from_millis(120));
        }
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datacraft_core::{DriverKind, SshTunnel};

    #[test]
    fn free_ports_are_distinct_and_usable() {
        let a = Tunnel::free_port().expect("port");
        let b = Tunnel::free_port().expect("port");
        assert!(a > 1024);
        let _ = b;
    }

    #[test]
    fn tunnel_refuses_a_profile_without_ssh() {
        let cfg = ConnectionConfig::sqlite("/tmp/x.db");
        assert!(Tunnel::start(&cfg, SshSecret::None).is_err());
    }

    #[test]
    fn tunnel_refuses_incomplete_settings() {
        let mut cfg = ConnectionConfig::new("pg", DriverKind::Postgres);
        cfg.ssh = SshTunnel {
            enabled: true,
            host: String::new(),
            port: 22,
            user: "ops".into(),
            key_file: String::new(),
            local_port: 0,
        };
        assert!(Tunnel::start(&cfg, SshSecret::None).is_err());
    }

    #[test]
    fn the_tunnel_hands_ssh_an_askpass_helper_or_its_own_stdin_never_the_secret() {
        let mut cfg = ConnectionConfig::new("pg", DriverKind::Postgres);
        cfg.host = "10.0.0.5".into();
        cfg.port = 5432;
        cfg.ssh = SshTunnel {
            enabled: true,
            host: "jump.example.com".into(),
            port: 2222,
            user: "ops".into(),
            key_file: String::new(),
            local_port: 0,
        };

        let quiet = spawn_plan(&cfg, 15432, &SshSecret::None);
        assert!(!quiet.inherit_stdin);
        assert!(quiet.env.is_empty());
        // Nothing to ask with, so ssh is kept from asking at all.
        assert!(quiet.args.contains(&"BatchMode=yes".into()));

        let asking = spawn_plan(&cfg, 15432, &SshSecret::Prompt);
        assert!(
            asking.inherit_stdin,
            "ssh answers its own prompt on our stdin"
        );
        assert!(asking.env.is_empty());
        assert!(!asking.args.contains(&"BatchMode=yes".into()));

        let keyed = spawn_plan(&cfg, 15432, &SshSecret::Keychain("local".into()));
        assert!(!keyed.inherit_stdin, "the helper needs no terminal");
        assert!(!keyed.args.contains(&"BatchMode=yes".into()));
        assert_eq!(
            keyed.env,
            vec![
                (
                    SSH_ASKPASS_ENV,
                    std::env::current_exe()
                        .expect("the test binary has a path")
                        .into_os_string()
                ),
                (SSH_ASKPASS_REQUIRE_ENV, OsString::from("force")),
                (ASKPASS_CONN_ENV, OsString::from("local")),
            ]
        );
    }

    #[test]
    fn a_stored_secret_becomes_the_helper_that_reads_it_back() {
        let (_guard, _log) = crate::test_support::fake_security("local#ssh", "hunter2");
        let cfg = ConnectionConfig::new("local", DriverKind::Postgres);
        if !crate::profiles::keychain_available() {
            // No keychain, no source for a helper: the tunnel falls back to ssh's own prompt.
            assert_ne!(
                SshSecret::for_connection("local"),
                SshSecret::Keychain("local".into())
            );
            return;
        }
        let secret = SshSecret::for_connection("local");
        assert_eq!(secret, SshSecret::Keychain("local".into()));
        let rendered = format!("{:?}", spawn_plan(&cfg, 15432, &secret));
        assert!(rendered.contains("local"), "{rendered}");
        assert!(
            !rendered.contains("hunter2"),
            "the secret must not reach an argument or the environment: {rendered}"
        );
    }

    #[test]
    fn the_facts_name_the_hop_and_the_port_that_was_bound() {
        // What `conn.test` reports as its `tunnel` object. The hop is the jump host, not the database:
        // the local port is the one `free_port()` actually bound.
        let child = Command::new("sleep")
            .arg("5")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("sleep");
        let tunnel = Tunnel {
            child,
            local_port: 15432,
            jump_host: "jump.example.com".into(),
            jump_port: 2222,
            remote: "10.0.0.5:5432".into(),
            command: "ssh -N -L 15432:10.0.0.5:5432 ops@jump.example.com".into(),
        };
        assert_eq!(
            tunnel_facts(&tunnel),
            serde_json::json!({ "host": "jump.example.com", "port": 2222, "local_port": 15432 })
        );
    }

    #[test]
    fn a_bad_jump_host_fails_with_a_message_not_a_hang() {
        let mut cfg = ConnectionConfig::new("pg", DriverKind::Postgres);
        cfg.host = "127.0.0.1".into();
        cfg.port = 1;
        cfg.connect_timeout_s = 5;
        cfg.ssh = SshTunnel {
            enabled: true,
            host: "127.0.0.1".into(),
            port: 1,
            user: "nobody".into(),
            key_file: String::new(),
            local_port: 0,
        };
        let result = Tunnel::start(&cfg, SshSecret::None);
        assert!(result.is_err());
    }
}
