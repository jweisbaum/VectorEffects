//! Durable job journal. Request cancellation never means job cancellation.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::PathBuf, sync::Mutex};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub key: String,
    pub operation: String,
    pub target: String,
    pub state: State,
    pub result: Option<Value>,
    pub error: Option<String>,
}
#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
    jobs: Mutex<Vec<Job>>,
}
impl Journal {
    pub fn open(path: PathBuf) -> Result<Self> {
        let mut jobs: Vec<Job> = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        for job in &mut jobs {
            if job.state == State::Running {
                job.state = State::Interrupted;
                job.error = Some("The app exited. Inspect the target before resuming; work was not silently retried.".into());
            }
        }
        let journal = Self {
            path,
            jobs: Mutex::new(jobs),
        };
        journal.save(
            &journal
                .jobs
                .lock()
                .map_err(|_| anyhow::anyhow!("Job lock failed"))?,
        )?;
        Ok(journal)
    }
    fn save(&self, jobs: &[Job]) -> Result<()> {
        crate::install::atomic_write(&self.path, &serde_json::to_vec_pretty(jobs)?)
    }
    /// Returns the existing record for an idempotency key. Never starts a duplicate job.
    pub fn start(&self, key: &str, operation: &str, target: &str) -> Result<(Job, bool)> {
        ensure!(
            !key.is_empty() && key.len() <= 256,
            "A bounded idempotency key is required"
        );
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("Job lock failed"))?;
        if let Some(job) = jobs.iter().find(|j| j.key == key) {
            ensure!(
                job.operation == operation && job.target == target,
                "Idempotency key belongs to another operation"
            );
            return Ok((job.clone(), false));
        }
        ensure!(
            jobs.len() < 10000,
            "Job history is full; archive finished records before starting more work"
        );
        let job = Job {
            id: uuid::Uuid::new_v4().to_string(),
            key: key.into(),
            operation: operation.into(),
            target: target.into(),
            state: State::Running,
            result: None,
            error: None,
        };
        let mut next = jobs.clone();
        next.push(job.clone());
        self.save(&next)?;
        *jobs = next;
        Ok((job, true))
    }
    pub fn finish(
        &self,
        id: &str,
        state: State,
        result: Option<Value>,
        error: Option<String>,
    ) -> Result<()> {
        ensure!(state != State::Running, "Finish requires a terminal state");
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("Job lock failed"))?;
        let mut next = jobs.clone();
        let job = next
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or_else(|| anyhow::anyhow!("Unknown job"))?;
        // Cancellation and completion race: the first durable terminal state wins.
        if job.state != State::Running {
            return Ok(());
        }
        job.state = state;
        job.result = result;
        job.error = error;
        if let Some(result) = &mut job.result {
            crate::policy::redact(result);
        }
        self.save(&next)?;
        *jobs = next;
        Ok(())
    }
    pub fn list(&self) -> Result<Vec<Job>> {
        Ok(self
            .jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("Job lock failed"))?
            .clone())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_cannot_be_overwritten_by_a_late_worker() {
        let dir = tempfile::tempdir().unwrap();
        let journal = Journal::open(dir.path().join("jobs.json")).unwrap();
        let (job, _) = journal.start("cancel", "weather", "boat").unwrap();
        journal
            .finish(&job.id, State::Cancelled, None, None)
            .unwrap();
        journal
            .finish(&job.id, State::Succeeded, None, None)
            .unwrap();
        assert_eq!(journal.list().unwrap()[0].state, State::Cancelled);
    }
    #[test]
    fn retries_and_crashes_do_not_repeat_mutations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let journal = Journal::open(path.clone()).unwrap();
        let (job, new) = journal.start("one", "export", "boat").unwrap();
        assert!(new);
        assert!(!journal.start("one", "export", "boat").unwrap().1);
        assert!(journal.start("one", "delete", "boat").is_err());
        drop(journal);
        let journal = Journal::open(path).unwrap();
        assert_eq!(journal.list().unwrap()[0].state, State::Interrupted);
        assert_eq!(journal.start("one", "export", "boat").unwrap().0.id, job.id);
    }
}
