//! Server-enforced workflow budgets. Only the app UI may create or expand a grant.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Grant {
    pub id: String,
    pub target: String,
    pub operations: Vec<String>,
    pub expires_unix_s: u64,
    pub budget_microusd: u64,
    pub reserved_microusd: u64,
    #[serde(default)]
    pub reservations: std::collections::BTreeMap<String, u64>,
}
#[derive(Debug)]
pub struct Grants {
    path: PathBuf,
    grants: Mutex<Vec<Grant>>,
}
impl Grants {
    pub fn open(path: PathBuf) -> Result<Self> {
        let grants = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            path,
            grants: Mutex::new(grants),
        })
    }
    pub fn approve_from_ui(
        &self,
        target: String,
        operations: Vec<String>,
        budget_microusd: u64,
        expires_unix_s: u64,
    ) -> Result<Grant> {
        ensure!(
            !target.is_empty() && !operations.is_empty(),
            "A target and operation scope are required"
        );
        let grant = Grant {
            id: uuid::Uuid::new_v4().to_string(),
            target,
            operations,
            expires_unix_s,
            budget_microusd,
            reserved_microusd: 0,
            reservations: Default::default(),
        };
        let mut grants = self
            .grants
            .lock()
            .map_err(|_| anyhow::anyhow!("Grant lock failed"))?;
        let mut next = grants.clone();
        next.push(grant.clone());
        crate::install::atomic_write(&self.path, &serde_json::to_vec(&next)?)?;
        *grants = next;
        Ok(grant)
    }
    /// Reserve before starting. A retry with the same action ID consumes the budget once.
    /// Unknown/unbounded estimates never qualify. Keep reservations on ambiguous failures.
    pub fn reserve(
        &self,
        target: &str,
        operation: &str,
        action_id: &str,
        cost_microusd: u64,
        now: u64,
    ) -> Result<Option<String>> {
        ensure!(!action_id.is_empty(), "An action ID is required");
        let mut grants = self
            .grants
            .lock()
            .map_err(|_| anyhow::anyhow!("Grant lock failed"))?;
        let mut next = grants.clone();
        for grant in &mut next {
            if grant.target != target
                || !grant.operations.iter().any(|op| op == operation)
                || grant.expires_unix_s <= now
            {
                continue;
            }
            if let Some(cost) = grant.reservations.get(action_id) {
                ensure!(*cost == cost_microusd, "An action's estimate changed");
                return Ok(Some(grant.id.clone()));
            }
            if cost_microusd
                > grant
                    .budget_microusd
                    .saturating_sub(grant.reserved_microusd)
            {
                continue;
            }
            grant.reserved_microusd += cost_microusd;
            grant.reservations.insert(action_id.into(), cost_microusd);
            let id = grant.id.clone();
            crate::install::atomic_write(&self.path, &serde_json::to_vec(&next)?)?;
            *grants = next;
            return Ok(Some(id));
        }
        Ok(None)
    }
    pub fn list(&self) -> Result<Vec<Grant>> {
        Ok(self
            .grants
            .lock()
            .map_err(|_| anyhow::anyhow!("Grant lock failed"))?
            .clone())
    }
    pub fn revoke_from_ui(&self, id: &str) -> Result<()> {
        let mut grants = self
            .grants
            .lock()
            .map_err(|_| anyhow::anyhow!("Grant lock failed"))?;
        let next: Vec<_> = grants.iter().filter(|g| g.id != id).cloned().collect();
        crate::install::atomic_write(&self.path, &serde_json::to_vec(&next)?)?;
        *grants = next;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_expiry_budget_and_retry_are_enforced_durably() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("grants.json");
        let grants = Grants::open(path.clone()).unwrap();
        grants
            .approve_from_ui("archive".into(), vec!["backfill".into()], 100, 1000)
            .unwrap();
        assert!(
            grants
                .reserve("other", "backfill", "a", 40, 1)
                .unwrap()
                .is_none()
        );
        assert!(
            grants
                .reserve("archive", "delete", "a", 40, 1)
                .unwrap()
                .is_none()
        );
        assert!(
            grants
                .reserve("archive", "backfill", "a", 40, 1000)
                .unwrap()
                .is_none()
        );
        assert!(
            grants
                .reserve("archive", "backfill", "a", 40, 1)
                .unwrap()
                .is_some()
        );
        assert!(
            grants
                .reserve("archive", "backfill", "a", 40, 1)
                .unwrap()
                .is_some()
        );
        drop(grants);
        let grants = Grants::open(path).unwrap();
        assert!(
            grants
                .reserve("archive", "backfill", "b", 61, 1)
                .unwrap()
                .is_none()
        );
        assert!(
            grants
                .reserve("archive", "backfill", "b", 60, 1)
                .unwrap()
                .is_some()
        );
    }
}
