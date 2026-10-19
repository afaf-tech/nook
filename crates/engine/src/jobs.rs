//! Automation: scheduled jobs (backup, transfer, sync, query, data generation) with a run log.
//! Navicat's Automation does the same work; here a job is plain JSON so `nook-cli job run`
//! and the in-app scheduler share it.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use nook_core::{Error, Result};
use serde::{Deserialize, Serialize};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Local UTC offset in minutes, read once from `date` so daily schedules fire at local wall time.
fn local_offset_minutes() -> i64 {
    static OFFSET: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFFSET.get_or_init(|| {
        std::process::Command::new("date")
            .arg("+%z")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| {
                let s = s.trim().to_string();
                let sign = if s.starts_with('-') { -1 } else { 1 };
                let digits = s.trim_start_matches(['+', '-']);
                if digits.len() < 4 {
                    return None;
                }
                let hours: i64 = digits[0..2].parse().ok()?;
                let minutes: i64 = digits[2..4].parse().ok()?;
                Some(sign * (hours * 60 + minutes))
            })
            .unwrap_or(0)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Query,
    Backup,
    Restore,
    Transfer,
    SyncData,
    SyncStructure,
    Datagen,
    Report,
    Alerts,
}

impl JobKind {
    pub fn label(self) -> &'static str {
        match self {
            JobKind::Query => "run query",
            JobKind::Backup => "backup",
            JobKind::Restore => "restore",
            JobKind::Transfer => "data transfer",
            JobKind::SyncData => "data synchronization",
            JobKind::SyncStructure => "structure synchronization",
            JobKind::Datagen => "generate data",
            JobKind::Report => "report",
            JobKind::Alerts => "alerts",
        }
    }
    pub fn parse(s: &str) -> Option<JobKind> {
        match s.to_ascii_lowercase().replace([' ', '-', '_'], "").as_str() {
            "query" | "runquery" | "sql" => Some(JobKind::Query),
            "backup" | "dump" => Some(JobKind::Backup),
            "restore" => Some(JobKind::Restore),
            "transfer" | "datatransfer" => Some(JobKind::Transfer),
            "syncdata" | "datasync" | "datasynchronization" => Some(JobKind::SyncData),
            "syncstructure" | "structuresync" | "structuresynchronization" => {
                Some(JobKind::SyncStructure)
            }
            "datagen" | "generate" | "generatedata" => Some(JobKind::Datagen),
            "report" => Some(JobKind::Report),
            "alerts" | "alert" => Some(JobKind::Alerts),
            _ => None,
        }
    }
    pub fn all() -> [JobKind; 9] {
        [
            JobKind::Query,
            JobKind::Backup,
            JobKind::Restore,
            JobKind::Transfer,
            JobKind::SyncData,
            JobKind::SyncStructure,
            JobKind::Datagen,
            JobKind::Report,
            JobKind::Alerts,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    Manual,
    EveryMinutes { minutes: u64 },
    Daily { hour: u8, minute: u8 },
}

impl Schedule {
    pub fn describe(&self) -> String {
        match self {
            Schedule::Manual => "manual".into(),
            Schedule::EveryMinutes { minutes } => {
                if *minutes >= 60 && minutes % 60 == 0 {
                    format!("every {}h", minutes / 60)
                } else {
                    format!("every {minutes}m")
                }
            }
            Schedule::Daily { hour, minute } => format!("daily at {hour:02}:{minute:02}"),
        }
    }

    /// `HH:MM` parsed into a daily schedule.
    pub fn parse(text: &str) -> Result<Schedule> {
        let text = text.trim();
        if text.is_empty() || text.eq_ignore_ascii_case("manual") {
            return Ok(Schedule::Manual);
        }
        if let Some(rest) = text.strip_prefix("every ") {
            let rest = rest.trim();
            let (value, unit) = rest.split_at(rest.len().saturating_sub(1));
            let n: u64 = value
                .trim()
                .parse()
                .map_err(|_| Error::config(format!("'{text}' is not a schedule")))?;
            let minutes = match unit {
                "m" => n,
                "h" => n * 60,
                "d" => n * 1440,
                _ => return Err(Error::config(format!("'{text}' is not a schedule"))),
            };
            if minutes == 0 {
                return Err(Error::config("an interval of zero minutes would spin"));
            }
            return Ok(Schedule::EveryMinutes { minutes });
        }
        let (h, m) = text
            .split_once(':')
            .ok_or_else(|| Error::config(format!("'{text}' is not a schedule")))?;
        let hour: u8 = h
            .trim()
            .parse()
            .map_err(|_| Error::config(format!("bad hour in '{text}'")))?;
        let minute: u8 = m
            .trim()
            .parse()
            .map_err(|_| Error::config(format!("bad minute in '{text}'")))?;
        if hour > 23 || minute > 59 {
            return Err(Error::config(format!("'{text}' is not a time of day")));
        }
        Ok(Schedule::Daily { hour, minute })
    }

    /// Unix time this schedule last came due at or before `at`.
    fn last_due(&self, at: u64) -> Option<u64> {
        match self {
            Schedule::Manual => None,
            Schedule::EveryMinutes { minutes } => {
                let step = (*minutes).max(1) * 60;
                Some(at - (at % step))
            }
            Schedule::Daily { hour, minute } => {
                let offset = local_offset_minutes() * 60;
                let local = at as i64 + offset;
                let day = local.div_euclid(86_400);
                let target_local = day * 86_400 + i64::from(*hour) * 3600 + i64::from(*minute) * 60;
                let target = target_local - offset;
                if target > at as i64 {
                    Some((target - 86_400) as u64)
                } else {
                    Some(target as u64)
                }
            }
        }
    }

    /// True when the job should run now, given when it last ran.
    pub fn due(&self, last_run: Option<u64>, at: u64) -> bool {
        let Some(due_at) = self.last_due(at) else {
            return false;
        };
        match last_run {
            None => true,
            Some(last) => last < due_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub kind: JobKind,
    /// Connection id the job reads from.
    #[serde(default)]
    pub conn: String,
    /// Second connection for transfer and sync jobs.
    #[serde(default)]
    pub target: String,
    /// Command parameters, exactly as the command registry expects them.
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default = "default_schedule")]
    pub schedule: Schedule,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub last_run: Option<u64>,
    #[serde(default)]
    pub last_status: Option<String>,
    #[serde(default)]
    pub runs: u64,
    #[serde(default)]
    pub failures: u64,
    /// Webhook POSTed the job's JSON status on failure. Empty means no notification.
    #[serde(default)]
    pub webhook: String,
    /// Jobs run after a successful run, in order. A chained failure stops the chain.
    #[serde(default)]
    pub then: Vec<String>,
}

fn default_schedule() -> Schedule {
    Schedule::Manual
}

fn default_enabled() -> bool {
    true
}

impl Job {
    pub fn new(id: impl Into<String>, name: impl Into<String>, kind: JobKind) -> Self {
        Job {
            id: id.into(),
            name: name.into(),
            kind,
            conn: String::new(),
            target: String::new(),
            params: serde_json::json!({}),
            schedule: Schedule::Manual,
            enabled: true,
            last_run: None,
            last_status: None,
            runs: 0,
            failures: 0,
            webhook: String::new(),
            then: Vec::new(),
        }
    }

    /// The command this job drives.
    pub fn command_id(&self) -> &'static str {
        match self.kind {
            JobKind::Query => "sql.run",
            JobKind::Backup => "backup.dump",
            JobKind::Restore => "backup.restore",
            JobKind::Transfer => "transfer.data",
            JobKind::SyncData => "sync.data",
            JobKind::SyncStructure => "sync.structure",
            JobKind::Datagen => "tools.datagen",
            JobKind::Report => "report.chart",
            // The rule set, not one rule: the command evaluates every enabled rule against the
            // metrics already collected and POSTs whatever tripped.
            JobKind::Alerts => "monitor.alerts",
        }
    }

    pub fn describe(&self) -> String {
        format!(
            "{} · {} · {} · {}{}",
            self.name,
            self.kind.label(),
            self.schedule.describe(),
            if self.enabled { "enabled" } else { "disabled" },
            match self.last_run {
                Some(t) => format!(" · last run {t}"),
                None => String::new(),
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobLogEntry {
    pub job: String,
    pub at: u64,
    pub ok: bool,
    pub message: String,
    pub ms: u128,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JobStore {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub jobs: Vec<Job>,
    #[serde(default)]
    pub log: Vec<JobLogEntry>,
    /// Path the store was loaded from, so `save` writes back to the same place.
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

impl JobStore {
    pub fn load() -> Result<Self> {
        let path = crate::profiles::jobs_path();
        if !path.is_file() {
            return Ok(JobStore {
                version: 1,
                path: Some(path),
                ..JobStore::default()
            });
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| Error::io(format!("read {}: {e}", path.display())))?;
        let mut store: JobStore = serde_json::from_str(&text)
            .map_err(|e| Error::config(format!("{}: {e}", path.display())))?;
        store.path = Some(path);
        let _ = store.retain_recent_log();
        Ok(store)
    }

    pub fn save(&self) -> Result<()> {
        let path = self.path.clone().unwrap_or_else(crate::profiles::jobs_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::io(format!("create {}: {e}", parent.display())))?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| Error::internal(format!("serialise jobs: {e}")))?;
        std::fs::write(&path, text)
            .map_err(|e| Error::io(format!("write {}: {e}", path.display())))?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id || j.name == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|j| j.id == id || j.name == id)
    }

    /// Ids as owned strings, for callers that need to look at every other job.
    pub fn ids(&self) -> Vec<String> {
        self.jobs.iter().map(|j| j.id.clone()).collect()
    }

    pub fn add(&mut self, job: Job) -> Result<()> {
        if job.id.trim().is_empty() || job.name.trim().is_empty() {
            return Err(Error::config("a job needs an id and a name"));
        }
        if self.get(&job.id).is_some() {
            return Err(Error::config(format!("job '{}' already exists", job.id)));
        }
        self.jobs.push(job);
        self.save()
    }

    pub fn upsert(&mut self, job: Job) -> Result<()> {
        match self.jobs.iter_mut().find(|j| j.id == job.id) {
            Some(slot) => *slot = job,
            None => self.jobs.push(job),
        }
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> Result<Job> {
        let index = self
            .jobs
            .iter()
            .position(|j| j.id == id || j.name == id)
            .ok_or_else(|| Error::not_found(format!("no job named '{id}'")))?;
        let job = self.jobs.remove(index);
        self.save()?;
        Ok(job)
    }

    pub fn ids_due(&self, at: u64) -> Vec<String> {
        self.jobs
            .iter()
            .filter(|j| j.enabled && j.schedule.due(j.last_run, at))
            .map(|j| j.id.clone())
            .collect()
    }

    pub fn record(&mut self, id: &str, ok: bool, message: impl Into<String>, ms: u128, at: u64) {
        let message = message.into();
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
            job.last_run = Some(at);
            job.last_status = Some(if ok {
                message.clone()
            } else {
                format!("failed: {message}")
            });
            job.runs += 1;
            if !ok {
                job.failures += 1;
            }
        }
        self.log.push(JobLogEntry {
            job: id.to_string(),
            at,
            ok,
            message,
            ms,
        });
        let _ = self.retain_recent_log();
        let _ = self.save();
    }

    /// Keep the log bounded: a job running every minute should not grow the file forever.
    pub fn retain_recent_log(&mut self) -> bool {
        const MAX: usize = 500;
        if self.log.len() > MAX {
            let drop = self.log.len() - MAX;
            self.log.drain(0..drop);
            true
        } else {
            false
        }
    }

    pub fn recent_log(&self, job: Option<&str>, limit: usize) -> Vec<&JobLogEntry> {
        self.log
            .iter()
            .rev()
            .filter(|e| job.map(|j| e.job == j).unwrap_or(true))
            .take(limit)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_parse_and_describe() {
        assert_eq!(Schedule::parse("manual").ok(), Some(Schedule::Manual));
        assert_eq!(
            Schedule::parse("every 15m").ok(),
            Some(Schedule::EveryMinutes { minutes: 15 })
        );
        assert_eq!(
            Schedule::parse("every 2h").ok(),
            Some(Schedule::EveryMinutes { minutes: 120 })
        );
        assert_eq!(
            Schedule::parse("03:30").ok(),
            Some(Schedule::Daily {
                hour: 3,
                minute: 30
            })
        );
        assert!(Schedule::parse("every 0m").is_err());
        assert!(Schedule::parse("25:00").is_err());
        assert!(Schedule::parse("soon").is_err());
        assert_eq!(
            Schedule::EveryMinutes { minutes: 60 }.describe(),
            "every 1h"
        );
    }

    #[test]
    fn interval_jobs_fire_once_per_interval() {
        let s = Schedule::EveryMinutes { minutes: 10 };
        assert!(s.due(None, 1_000_000));
        // Same ten-minute bucket: not due again.
        assert!(!s.due(Some(1_000_000), 1_000_060));
        // Next bucket.
        assert!(s.due(Some(1_000_000), 1_000_600 + 1));
        assert!(!Schedule::Manual.due(None, 1_000_000));
    }

    #[test]
    fn daily_jobs_fire_after_the_time_of_day() {
        let s = Schedule::Daily {
            hour: 8,
            minute: 30,
        };
        let offset = local_offset_minutes() * 60;
        let local_0830 = 20_000u64 * 86_400 + 8 * 3600 + 30 * 60;
        let at = (local_0830 as i64 - offset) as u64;
        assert!(s.due(None, at));
        assert!(s.due(Some(at - 86_400), at + 60));
        assert!(!s.due(Some(at), at + 60));
        // Just before the time of day, the last due point is yesterday.
        assert!(s.due(Some(at - 86_400 - 1), at - 60));
        assert!(!s.due(Some(at - 60), at - 60));
    }

    #[test]
    fn store_keeps_the_log_bounded_and_records_runs() {
        let _guard = crate::test_support::scratch_config();
        let mut store = JobStore::default();
        let job = Job::new("nightly", "Nightly backup", JobKind::Backup);
        assert_eq!(job.command_id(), "backup.dump");
        store.jobs.push(job);
        for i in 0..600 {
            store.record("nightly", i % 2 == 0, format!("run {i}"), 5, i as u64);
        }
        assert_eq!(store.log.len(), 500);
        let job = store.get("nightly").expect("job");
        assert_eq!(job.runs, 600);
        assert_eq!(job.failures, 300);
        assert!(job.last_status.as_deref().unwrap_or("").contains("failed"));
        assert_eq!(store.recent_log(Some("nightly"), 3).len(), 3);
        assert!(
            store.ids_due(now()).is_empty(),
            "manual jobs never come due"
        );
    }

    #[test]
    fn duplicate_and_missing_jobs_are_errors() {
        let _guard = crate::test_support::scratch_config();
        let mut store = JobStore::default();
        let path_less = {
            let mut s = store.clone();
            s.path = None;
            s
        };
        store = path_less;
        let job = Job::new("a", "A", JobKind::Query);
        store.jobs.push(job.clone());
        assert!(store.get("a").is_some());
        assert!(store.get("missing").is_none());
        assert!(store.remove("missing").is_err());
    }

    use super::*;

    #[test]
    fn every_kind_maps_to_a_command_that_exists() {
        // A job kind whose command id is not registered is a scheduler that cannot run it, which is
        // what `monitor.alerts` was until its help text was checked against the registry.
        let registry = crate::registry();
        for kind in JobKind::all() {
            let job = Job::new("probe", "probe", kind);
            let id = job.command_id();
            assert!(
                registry.iter().any(|c| c.id == id),
                "{kind:?} drives {id}, which is not a registered command"
            );
        }
    }

    #[test]
    fn the_create_help_lists_every_kind() {
        // `job.create`'s help is the only place a user learns which kinds exist, and a static string
        // cannot be derived from the enum. Adding a kind without naming it there is how `alerts`
        // shipped undocumented.
        let help = crate::registry()
            .iter()
            .find(|c| c.id == "job.create")
            .expect("job.create is registered")
            .help;
        // The name a caller types, which is the enum's own snake_case name — not the label, which is
        // prose ("run query") for the job list.
        for kind in JobKind::all() {
            let name = match kind {
                JobKind::Query => "query",
                JobKind::Backup => "backup",
                JobKind::Restore => "restore",
                JobKind::Transfer => "transfer",
                JobKind::SyncData => "sync_data",
                JobKind::SyncStructure => "sync_structure",
                JobKind::Datagen => "datagen",
                JobKind::Report => "report",
                JobKind::Alerts => "alerts",
            };
            assert!(
                help.contains(name),
                "job.create's help does not mention {name:?}: {help}"
            );
        }
    }

    #[test]
    fn a_kind_survives_a_round_trip_through_its_own_name() {
        for kind in JobKind::all() {
            assert_eq!(JobKind::parse(kind.label()), Some(kind), "{kind:?}");
        }
    }
}
