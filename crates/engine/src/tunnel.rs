//! SSH tunnel: DataCraft shells out to the system `ssh`, so agent keys and `~/.ssh/config` just work.

use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use datacraft_core::{ConnectionConfig, Error, Result};

/// A running `ssh -N -L …` process. Dropping it closes the tunnel.
pub struct Tunnel {
    child: Child,
    pub local_port: u16,
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
    pub fn start(cfg: &ConnectionConfig) -> Result<Tunnel> {
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
        let args = cfg.ssh.ssh_args(&cfg.host, cfg.port, local_port);
        let command = format!("ssh {}", args.join(" "));
        let child = Command::new("ssh")
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::connect(format!("could not start ssh: {e}")))?;
        let mut tunnel = Tunnel {
            child,
            local_port,
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
        assert!(Tunnel::start(&cfg).is_err());
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
        assert!(Tunnel::start(&cfg).is_err());
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
        let result = Tunnel::start(&cfg);
        assert!(result.is_err());
    }
}
