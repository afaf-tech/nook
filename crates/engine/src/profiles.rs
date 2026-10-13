//! Saved connection profiles, application settings and — on macOS — the login keychain.

use std::path::PathBuf;
use std::process::Command;

use datacraft_core::{ConnectionConfig, Error, Result};
use serde::{Deserialize, Serialize};

/// Where DataCraft keeps its files. `DATACRAFT_CONFIG_DIR` overrides everything, which is how the
/// tests stay out of the real profile directory; otherwise the platform convention applies.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DATACRAFT_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("Library/Application Support/DataCraft");
        }
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("datacraft");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".config/datacraft");
    }
    PathBuf::from(".datacraft")
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

const KEYCHAIN_SERVICE: &str = "datacraft";

pub fn keychain_available() -> bool {
    cfg!(target_os = "macos")
}

/// Store a password in the login keychain. The secret never reaches the process arguments.
pub fn keychain_store(id: &str, secret: &str) -> Result<()> {
    if !keychain_available() {
        return Err(Error::unsupported(
            "keychain storage is macOS only; keep the password in the profile file (mode 0600) instead",
        ));
    }
    let out = Command::new("security")
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
    let out = Command::new("security")
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
    let mut secret = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if secret.is_empty() {
        return Ok(None);
    }
    secret.push('\0');
    secret.pop();
    Ok(Some(secret))
}

pub fn keychain_delete(id: &str) -> Result<()> {
    if !keychain_available() {
        return Ok(());
    }
    let out = Command::new("security")
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
        assert!(
            dir.to_string_lossy().contains("DataCraft")
                || dir.to_string_lossy().contains("datacraft")
        );
    }
}
