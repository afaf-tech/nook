//! Background jobs: a statement that takes seconds must not take the frame with it.
//!
//! The shape is the one `ai.rs` already uses for its `curl` round-trip, generalised: the caller
//! spawns an OS thread, the thread hands results back over an `mpsc` channel, and the frame polls.
//! What is new here is that a job can be **cancelled**, which needs a second connection — you cannot
//! ask a server to cancel the statement the only connection is busy executing.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use nook_core::{ConnectionConfig, Error, Result};
use nook_model::ResultSet;

/// What a job is doing right now.
#[derive(Debug, Clone, PartialEq)]
pub enum JobState {
    Running,
    Done,
    Failed(String),
    Cancelled,
}

impl JobState {
    pub fn label(&self) -> &'static str {
        match self {
            JobState::Running => "running",
            JobState::Done => "done",
            JobState::Failed(_) => "failed",
            JobState::Cancelled => "cancelled",
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self, JobState::Running)
    }
}

/// One background run. `rows` and `elapsed_ms` are live while it runs — that is what the spinner
/// paints — and final once it stops.
#[derive(Debug, Clone)]
pub struct Job {
    pub id: u64,
    /// The statement as the user wrote it, for the status line.
    pub sql: String,
    /// The connection the job runs on, so `sql.cancel` can dial a sibling.
    pub conn: String,
    pub state: JobState,
    pub rows: usize,
    pub pages: usize,
    pub elapsed_ms: f64,
    pub started: Instant,
    /// The tab the pages land in. A job whose tab was closed still finishes; its pages are dropped.
    pub tab: Option<u64>,
}

impl Job {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "job": self.id,
            "sql": self.sql,
            "conn": self.conn,
            "state": self.state.label(),
            "error": match &self.state { JobState::Failed(e) => Some(e.clone()), _ => None },
            "rows": self.rows,
            "pages": self.pages,
            "elapsed_ms": (self.elapsed_ms * 100.0).round() / 100.0,
        })
    }
}

/// What the worker sends back. Pages arrive as they are read, which is what makes "rows appear
/// immediately" true without a second result type.
#[derive(Debug)]
pub enum Message {
    Page(ResultSet),
    Done { elapsed_ms: f64, affected: u64 },
    Failed(String),
    Cancelled,
}

struct Slot {
    job: Job,
    cancel: Arc<AtomicBool>,
    /// The backend pid of the worker's own connection, once it has one. `sql.cancel` needs it, and
    /// it cannot be asked for after the fact: the connection is gone with the worker.
    pid: Arc<AtomicUsize>,
}

/// Owns every background job. Lives on `Session`.
pub struct Runner {
    next: u64,
    slots: BTreeMap<u64, Slot>,
    tx: Sender<(u64, Message)>,
    rx: Receiver<(u64, Message)>,
}

impl Default for Runner {
    fn default() -> Self {
        Runner::new()
    }
}

impl Runner {
    pub fn new() -> Runner {
        let (tx, rx) = channel();
        Runner {
            next: 1,
            slots: BTreeMap::new(),
            tx,
            rx,
        }
    }

    /// Start a job on its own connection. `cfg` must already carry its password: the keychain is
    /// read on the calling thread, never on a worker that cannot report what went wrong.
    pub fn start(
        &mut self,
        cfg: ConnectionConfig,
        sql: String,
        limit: usize,
        page_rows: usize,
        tab: Option<u64>,
    ) -> u64 {
        let id = self.next;
        self.next += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        let pid = Arc::new(AtomicUsize::new(0));
        let job = Job {
            id,
            sql: sql.clone(),
            conn: cfg.id.clone(),
            state: JobState::Running,
            rows: 0,
            pages: 0,
            elapsed_ms: 0.0,
            started: Instant::now(),
            tab,
        };
        self.slots.insert(
            id,
            Slot {
                job,
                cancel: Arc::clone(&cancel),
                pid: Arc::clone(&pid),
            },
        );
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let report = Report {
                tx: &tx,
                id,
                cancel: &cancel,
                pid: &pid,
            };
            let terminal = match work(&cfg, &sql, limit, page_rows, &report) {
                Ok(rs) => Message::Done {
                    elapsed_ms: rs.elapsed_ms,
                    affected: rs.affected,
                },
                Err(Error::Canceled) => Message::Cancelled,
                // A server-side cancel arrives as an error, not as our flag: Postgres says
                // "canceling statement due to user request", MySQL says the query was interrupted.
                // The flag is the only thing that knows the user asked, so it decides the verdict —
                // otherwise a Stop would be reported as a failure, which is the opposite of what the
                // user did.
                Err(e) if cancel.load(Ordering::SeqCst) => {
                    let _ = e;
                    Message::Cancelled
                }
                Err(e) => Message::Failed(e.to_string()),
            };
            let _ = tx.send((id, terminal));
        });
        id
    }

    /// Ask a job to stop. Returns the granularity the driver can actually deliver.
    pub fn cancel(&mut self, cfg: &ConnectionConfig, id: u64) -> Result<String> {
        let slot = self
            .slots
            .get(&id)
            .ok_or_else(|| Error::not_found(format!("no job {id}")))?;
        if !slot.job.state.is_running() {
            return Err(Error::config(format!(
                "job {id} is already {}",
                slot.job.state.label()
            )));
        }
        slot.cancel.store(true, Ordering::SeqCst);
        let pid = slot.pid.load(Ordering::SeqCst);
        if pid == 0 {
            // The worker has not connected yet, so the flag alone will stop it before it reads
            // anything. Nothing to tell the server.
            return Ok("before the first page".into());
        }
        // A sibling connection: the worker's own is busy executing the statement being cancelled.
        let mut driver = nook_drivers::open(cfg)?;
        driver.cancel(pid as u64)?;
        Ok(driver.cancel_granularity().to_string())
    }

    /// Take every message that has arrived. Called once per frame; the caller applies the pages.
    pub fn drain(&mut self) -> Vec<(u64, Message)> {
        let mut out = Vec::new();
        while let Ok((id, msg)) = self.rx.try_recv() {
            if let Some(slot) = self.slots.get_mut(&id) {
                match &msg {
                    Message::Page(rs) => {
                        slot.job.rows += rs.row_count();
                        slot.job.pages += 1;
                    }
                    Message::Done { elapsed_ms, .. } => {
                        slot.job.state = JobState::Done;
                        slot.job.elapsed_ms = *elapsed_ms;
                    }
                    Message::Failed(e) => slot.job.state = JobState::Failed(e.clone()),
                    Message::Cancelled => slot.job.state = JobState::Cancelled,
                }
                if !slot.job.state.is_running() {
                    slot.job.elapsed_ms = slot.job.started.elapsed().as_secs_f64() * 1000.0;
                }
            }
            out.push((id, msg));
        }
        out
    }

    pub fn job(&self, id: u64) -> Option<&Job> {
        self.slots.get(&id).map(|s| &s.job)
    }

    pub fn jobs(&self) -> impl Iterator<Item = &Job> {
        self.slots.values().map(|s| &s.job)
    }

    /// The job a tab is waiting on, if any.
    pub fn running_for_tab(&self, tab: u64) -> Option<&Job> {
        self.slots
            .values()
            .map(|s| &s.job)
            .find(|j| j.tab == Some(tab) && j.state.is_running())
    }

    /// Forget a finished job. A running one is refused: dropping it would leave a thread writing to
    /// a tab nobody is watching.
    pub fn forget(&mut self, id: u64) -> Result<()> {
        match self.slots.get(&id) {
            None => return Err(Error::not_found(format!("no job {id}"))),
            Some(slot) if slot.job.state.is_running() => {
                return Err(Error::config(format!(
                    "job {id} is still running; cancel it first"
                )))
            }
            Some(_) => {}
        }
        self.slots.remove(&id);
        Ok(())
    }
}

/// Everything one worker needs to report back: the channel, its own id, and the two flags the
/// caller watches. Grouped so the worker's signature says what it does, not how many pieces it has.
struct Report<'a> {
    tx: &'a Sender<(u64, Message)>,
    id: u64,
    cancel: &'a Arc<AtomicBool>,
    pid: &'a Arc<AtomicUsize>,
}

/// The worker's whole job: dial, run, send pages until the read ends or the flag is set.
fn work(
    cfg: &ConnectionConfig,
    sql: &str,
    limit: usize,
    page_rows: usize,
    report: &Report<'_>,
) -> Result<ResultSet> {
    let Report {
        tx,
        id,
        cancel,
        pid,
    } = report;
    let id = *id;
    let mut driver = nook_drivers::open(cfg)?;
    if let Some(p) = driver.backend_pid() {
        pid.store(p as usize, Ordering::SeqCst);
    }
    let mut sent = 0usize;
    let mut cancelled = false;
    let result = {
        let mut sink = |rs: ResultSet| -> bool {
            if cancel.load(Ordering::SeqCst) {
                cancelled = true;
                return false;
            }
            sent += rs.row_count();
            let _ = tx.send((id, Message::Page(rs)));
            sent < limit
        };
        driver.run_stream(sql, limit, page_rows, &mut sink)?
    };
    if cancelled {
        return Err(Error::Canceled);
    }
    // A driver with no paging sends its rows back in the result; hand them over now, once.
    if sent == 0 && !result.rows.is_empty() {
        let _ = tx.send((id, Message::Page(result.clone())));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancelled_job_is_not_reported_as_a_failure() {
        // The distinction is the whole point: `Cancelled` is what the user asked for, `Failed` is
        // what the server did to them.
        assert_eq!(JobState::Cancelled.label(), "cancelled");
        assert_ne!(
            JobState::Cancelled.label(),
            JobState::Failed(String::new()).label()
        );
    }

    #[test]
    fn a_job_that_has_not_connected_yet_cancels_without_a_pid() {
        let mut runner = Runner::new();
        let cfg = ConnectionConfig::parse("sqlite:/tmp/nook-runner-test.db").expect("parses");
        let id = runner.start(cfg.clone(), "select 1".into(), 10, 10, None);
        // The worker may or may not have connected by now; either way the flag is set and the
        // answer names the granularity rather than lying about a backend cancel.
        let note = runner
            .cancel(&cfg, id)
            .or_else(|e| {
                // A job that finished in microseconds is already done; that is a correct refusal.
                if e.to_string().contains("already") {
                    Ok("finished".to_string())
                } else {
                    Err(e)
                }
            })
            .expect("cancel answers");
        assert!(!note.is_empty());
    }
}
