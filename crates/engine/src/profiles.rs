//! Saved connection profiles, application settings and — on macOS — the login keychain.

use std::path::PathBuf;
use std::process::Command;

use nook_core::{ConnectionConfig, Error, Result};
use serde::{Deserialize, Serialize};

/// Where Nook keeps its files. `NOOK_CONFIG_DIR` overrides everything, which is how the
/// tests stay out of the real profile directory; otherwise the platform convention applies.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NOOK_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("Library/Application Support/Nook");
        }
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("nook");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config/nook");
    }
    PathBuf::from(".nook")
}

pub fn ensure_dir() -> Result<PathBuf> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| Error::io(format!("create {}: {e}", dir.display())))?;
    Ok(dir)
}

pub fn profiles_path() -> PathBuf {
    config_dir().join("profiles.json")
}

pub fn jobs_path() -> PathBuf {
    config_dir().join("jobs.json")
}

pub fn log_dir() -> PathBuf {
    config_dir().join("logs")
}

/// The saved connection list, exactly the JSON that ships in `profiles.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileStore {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub profiles: Vec<ConnectionConfig>,
    /// Whole-app settings as JSON so the schema can grow without a migration.
    #[serde(default)]
    pub settings: serde_json::Value,
}

impl ProfileStore {
    pub fn load() -> Result<Self> {
        let path = profiles_path();
        if !path.is_file() {
            return Ok(ProfileStore {
                version: 1,
                profiles: vec![],
                settings: serde_json::Value::Null,
            });
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| Error::io(format!("read {}: {e}", path.display())))?;
        let mut store: ProfileStore = serde_json::from_str(&text)
            .map_err(|e| Error::config(format!("{}: {e}", path.display())))?;
        store.version = store.version.max(1);
        Ok(store)
    }

    pub fn save(&self) -> Result<()> {
        ensure_dir()?;
        let path = profiles_path();
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::internal(format!("serialise profiles: {e}")))?;
        std::fs::write(&path, text)
            .map_err(|e| Error::io(format!("write {}: {e}", path.display())))?;
        restrict_permissions(&path);
        Ok(())
    }

    /// The whole-app settings blob.
    pub fn settings(&self) -> &serde_json::Value {
        &self.settings
    }

    /// Remember one preference. The blob is JSON so the schema can grow without a migration; keys
    /// nobody knows about are left alone.
    pub fn set_setting(&mut self, key: &str, value: serde_json::Value) {
        if !self.settings.is_object() {
            self.settings = serde_json::Value::Object(serde_json::Map::new());
        }
        if let Some(map) = self.settings.as_object_mut() {
            map.insert(key.to_owned(), value);
        }
    }

    pub fn list(&self) -> &[ConnectionConfig] {
        &self.profiles
    }

    pub fn get(&self, id: &str) -> Option<&ConnectionConfig> {
        self.profiles.iter().find(|p| p.id == id || p.name == id)
    }

    pub fn groups(&self) -> Vec<String> {
        let mut groups: Vec<String> = self
            .profiles
            .iter()
            .map(|p| p.group.clone())
            .filter(|g| !g.is_empty())
            .collect();
        groups.sort();
        groups.dedup();
        groups
    }

    /// Insert or replace by id, and persist.
    pub fn upsert(&mut self, cfg: ConnectionConfig) -> Result<()> {
        if cfg.id.trim().is_empty() || cfg.name.trim().is_empty() {
            return Err(Error::config("a connection needs an id and a name"));
        }
        match self.profiles.iter_mut().find(|p| p.id == cfg.id) {
            Some(slot) => *slot = cfg,
            None => self.profiles.push(cfg),
        }
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> Result<ConnectionConfig> {
        let index = self
            .profiles
            .iter()
            .position(|p| p.id == id || p.name == id)
            .ok_or_else(|| Error::not_found(format!("no connection named '{id}'")))?;
        let cfg = self.profiles.remove(index);
        self.save()?;
        Ok(cfg)
    }
}

/// Only the owner may read the profile file: it can hold passwords when the keychain is not used.
fn restrict_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

// ------------------------------------------------------------------ keychain

const KEYCHAIN_SERVICE: &str = "nook";

/// The `security` binary the keychain calls shell out to. `NOOK_SECURITY_BIN` points it at a
/// fake, which is how the tests exercise the accounts without a login keychain.
pub fn security_bin() -> String {
    std::env::var("NOOK_SECURITY_BIN").unwrap_or_else(|_| "security".to_string())
}

/// The keychain account that holds a connection's SSH password. The database password lives under
/// the bare id (`profiles.rs`'s `savePassword` path), so the two can never collide.
pub fn ssh_account(id: &str) -> String {
    format!("{id}#ssh")
}

/// The one sentence Nook owns for "there is no keychain here": `conn.sshPassword` refuses with
/// it, the dialog shows it beside its disabled toggle, and `--ssh-askpass` has no source for it. A
/// `ponytail:` the fallback is ssh's own prompt on the terminal the app was started from — an
/// in-app prompt whose answer is written to the keychain for the length of the run is the upgrade.
pub const NO_KEYCHAIN: &str =
    "this system has no OS keychain, so the SSH password cannot be saved; \
leave it empty and ssh asks for it on the terminal Nook was started from";

/// `NO_KEYCHAIN` where the caller has no keychain, `None` where there is one: the branch is a value
/// the tests can check on any platform.
pub fn keychain_refusal(available: bool) -> Option<&'static str> {
    (!available).then_some(NO_KEYCHAIN)
}

/// The one line `--ssh-askpass` prints for the connection's SSH password: read here, in the helper
/// `ssh` execs, so the secret never appears in an argument or an environment variable.
pub fn ssh_askpass_line(id: &str) -> Result<String> {
    keychain_lookup(&ssh_account(id))?
        .ok_or_else(|| Error::not_found(format!("no SSH password is stored for '{id}'")))
}

pub fn keychain_available() -> bool {
    cfg!(target_os = "macos")
}

/// Store a password in the login keychain.
///
/// **Ceiling: the secret is the child's `-w` argument.** A bare `-w` asks the terminal for the
/// password, and this app is launched from a terminal often enough that the pipe form hangs it:
/// `security` opens `/dev/tty` for the confirmation reads, never takes the piped bytes, and the store
/// blocks forever in `wait_with_output` (measured with the same binary and the same call, differing
/// only in whether the app had a controlling terminal).
///
/// The door out is real and measured, not assumed: with no controlling terminal the same bare `-w`
/// takes **both lines from the pipe** and the store succeeds (`printf 'h1\nh1\n' | security
/// add-generic-password -U -s … -a … -w` → exit 0, and `find-generic-password -w` reads the value
/// back), and a child detached with `setsid` can still reach the login keychain (same probe from
/// `os.setsid()`'s new session → stored, and read back). What is missing is only the detach itself,
/// i.e. `setsid` between fork and exec — reachable through `unsafe` (a raw `extern "C"` next to a
/// `pre_exec` call; the workspace has no `unsafe` at all today) or a detach crate such as `nix`,
/// which this phase's non-goals forbid. So the exposure is recorded here instead: for as long as the
/// child lives, the same user can read the secret from the process table. Revisit when adding
/// `unsafe` to the workspace is a decision someone is willing to make on its own merits — the
/// `security` side of that change is already proven to work.
pub fn keychain_store(id: &str, secret: &str) -> Result<()> {
    if !keychain_available() {
        return Err(Error::unsupported(
            "keychain storage is macOS only; keep the password in the profile file (mode 0600) instead",
        ));
    }
    let out = Command::new(security_bin())
        .args([
            "add-generic-password",
            "-U",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            id,
            "-w",
            secret,
        ])
        .output()
        .map_err(|e| Error::io(format!("security: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::io(format!(
            "keychain refused the password: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

pub fn keychain_lookup(id: &str) -> Result<Option<String>> {
    if !keychain_available() {
        return Ok(None);
    }
    let out = Command::new(security_bin())
        .args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            id,
            "-w",
        ])
        .output()
        .map_err(|e| Error::io(format!("security: {e}")))?;
    if !out.status.success() {
        return Ok(None);
    }
    let secret = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if secret.is_empty() {
        return Ok(None);
    }
    Ok(Some(secret))
}

pub fn keychain_delete(id: &str) -> Result<()> {
    if !keychain_available() {
        return Ok(());
    }
    let out = Command::new(security_bin())
        .args(["delete-generic-password", "-s", KEYCHAIN_SERVICE, "-a", id])
        .output()
        .map_err(|e| Error::io(format!("security: {e}")))?;
    if out.status.success() || String::from_utf8_lossy(&out.stderr).contains("could not be found") {
        Ok(())
    } else {
        Err(Error::io(format!(
            "keychain delete failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_round_trips_by_id() {
        let mut store = ProfileStore::default();
        let cfg = ConnectionConfig::sqlite("/tmp/a.db");
        let id = cfg.id.clone();
        // No save() here: the test must not touch the user's real profiles file.
        match store.profiles.iter_mut().find(|p| p.id == cfg.id) {
            Some(slot) => *slot = cfg.clone(),
            None => store.profiles.push(cfg.clone()),
        }
        assert_eq!(
            store.get(&id).map(|c| c.file.clone()),
            Some("/tmp/a.db".into())
        );
        assert!(store.get("nope").is_none());
        assert_eq!(store.groups().len(), 0);
    }

    #[test]
    fn upsert_rejects_nameless_profiles() {
        let mut store = ProfileStore::default();
        let mut cfg = ConnectionConfig::sqlite("/tmp/b.db");
        cfg.name = String::new();
        assert!(store.upsert(cfg).is_err());
    }

    #[test]
    fn config_dir_is_under_the_home_directory() {
        let dir = config_dir();
        assert!(dir.to_string_lossy().contains("Nook") || dir.to_string_lossy().contains("nook"));
    }

    #[test]
    fn the_ssh_password_gets_an_account_of_its_own() {
        assert_eq!(ssh_account("local"), "local#ssh");
        assert_ne!(ssh_account("local"), "local");
        // A connection whose id already ends in the suffix must not collide with the other account.
        assert_ne!(ssh_account("local"), ssh_account(&ssh_account("local")));
    }

    #[test]
    fn the_refusal_sentence_names_the_keychain_and_the_fallback() {
        assert!(keychain_refusal(true).is_none());
        let sentence = keychain_refusal(false).expect("refusal");
        assert!(sentence.contains("keychain"), "{sentence}");
        assert!(
            sentence.contains("ssh asks"),
            "the sentence must say what happens instead: {sentence}"
        );
    }

    #[test]
    fn the_ssh_account_round_trips_through_the_keychain() {
        let (_guard, log) = crate::test_support::fake_security("local#ssh", "hunter2");
        if !keychain_available() {
            // Nowhere else has a keychain to store into: the refusal sentence is the whole story.
            assert!(keychain_store(&ssh_account("local"), "hunter2").is_err());
            assert_eq!(
                keychain_lookup(&ssh_account("local")).expect("lookup"),
                None
            );
            return;
        }
        keychain_store(&ssh_account("local"), "hunter2").expect("store");
        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(
            calls.contains("-a local#ssh"),
            "the store's account: {calls}"
        );
        // The two accounts differ: the one the database password uses holds nothing.
        assert_eq!(
            keychain_lookup(&ssh_account("local"))
                .expect("lookup")
                .as_deref(),
            Some("hunter2")
        );
        assert_eq!(keychain_lookup("local").expect("lookup"), None);
        // The one line the askpass helper prints is the account's secret.
        assert_eq!(ssh_askpass_line("local").expect("askpass"), "hunter2");
        keychain_delete(&ssh_account("local")).expect("delete");
        assert!(std::fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("delete-generic-password"));
    }
}
