//! Named resource leases for task arbitration.
//!
//! `inference` reuses `AppState.operation` (capacity 1) so every existing
//! try_lock caller keeps its semantics — queued tasks wait for the same lock
//! instead of racing it. Other slots are bookkeeping-only: `browser` (the one
//! shared agent tab), `workspace-write:<canonical root>` (one writer per
//! root — task worktrees get distinct roots by construction), and
//! `background` (bounded background jobs). Permits are RAII: cancel, panic,
//! and normal completion all release.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

pub const INFERENCE: &str = "inference";
pub const BROWSER: &str = "browser";
pub const BACKGROUND: &str = "background";

pub fn workspace_write(root: &str) -> String {
    format!(
        "workspace-write:{}",
        root.replace('\\', "/").trim_end_matches('/').to_lowercase()
    )
}

fn capacity(name: &str) -> usize {
    if name == BACKGROUND { 2 } else { 1 }
}

#[derive(Default)]
pub struct LeaseRegistry {
    holders: Mutex<HashMap<String, Vec<(String, String)>>>,
    wake: tokio::sync::Notify,
}

/// Why an acquire failed or is still waiting — a holder's reason when known,
/// or a generic note for slots claimed outside the registry (the inference
/// slot is also held directly by operations that predate the task queue).
#[derive(Debug)]
pub struct LeaseBusy {
    pub reason: String,
}

impl LeaseRegistry {
    /// Who holds the slot now, for "waiting on" display.
    pub fn holder(&self, name: &str) -> Option<(String, String)> {
        self.holders.lock().ok()?.get(name)?.first().cloned()
    }

    fn claim(
        self: &Arc<Self>,
        name: &str,
        owner: &str,
        reason: &str,
    ) -> Result<LeasePermit, LeaseBusy> {
        let mut holders = self.holders.lock().map_err(|_| LeaseBusy {
            reason: "lease registry unavailable".into(),
        })?;
        let held = holders.entry(name.to_string()).or_default();
        if held.len() >= capacity(name) {
            let reason = held
                .first()
                .map(|(_, r)| r.clone())
                .unwrap_or_else(|| "in use".into());
            return Err(LeaseBusy { reason });
        }
        held.push((owner.to_string(), reason.to_string()));
        Ok(LeasePermit {
            registry: Arc::downgrade(self),
            name: name.to_string(),
            owner: owner.to_string(),
        })
    }

    pub fn try_acquire(
        self: &Arc<Self>,
        name: &str,
        owner: &str,
        reason: &str,
    ) -> Result<LeasePermit, LeaseBusy> {
        self.claim(name, owner, reason)
    }

    /// Wait for a slot until `timeout` (None = forever, checking `still_valid`
    /// each tick so a cancelled task stops waiting). `on_wait` runs about
    /// every 100 ms with the current holder's reason for status updates.
    pub async fn acquire(
        self: &Arc<Self>,
        name: &str,
        owner: &str,
        reason: &str,
        timeout: Option<Duration>,
        mut still_valid: impl FnMut() -> bool,
        mut on_wait: impl FnMut(&str),
    ) -> Result<LeasePermit, LeaseBusy> {
        let start = Instant::now();
        loop {
            match self.claim(name, owner, reason) {
                Ok(permit) => return Ok(permit),
                Err(busy) => {
                    if timeout.is_some_and(|limit| start.elapsed() >= limit) {
                        return Err(busy);
                    }
                    if !still_valid() {
                        return Err(LeaseBusy {
                            reason: "cancelled".into(),
                        });
                    }
                    on_wait(&busy.reason);
                    tokio::select! {
                        _ = self.wake.notified() => {}
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                    }
                }
            }
        }
    }

    fn release(&self, name: &str, owner: &str) {
        if let Ok(mut holders) = self.holders.lock() {
            if let Some(held) = holders.get_mut(name) {
                held.retain(|(o, _)| o != owner);
                if held.is_empty() {
                    holders.remove(name);
                }
            }
        }
        self.wake.notify_one();
    }
}

/// RAII release. A task dropped mid-wait, cancelled, or panicking still frees
/// the slot for the next holder.
#[derive(Debug)]
pub struct LeasePermit {
    registry: Weak<LeaseRegistry>,
    name: String,
    owner: String,
}

impl LeasePermit {
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for LeasePermit {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.release(&self.name, &self.owner);
        }
    }
}

/// The inference lease: the operation mutex (the real exclusion) plus a
/// registry permit so waiters can name the current owner. Holders through
/// this path appear as the reason; other `operation` holders show as a
/// generic "another operation".
pub struct InferenceLease {
    _operation: tokio::sync::OwnedMutexGuard<()>,
    _permit: LeasePermit,
}

/// Try to take the inference lease now. `Err` names the current holder.
pub fn try_acquire_inference(
    state: &crate::AppState,
    owner: &str,
    reason: &str,
) -> Result<InferenceLease, LeaseBusy> {
    match state.operation.clone().try_lock_owned() {
        Ok(guard) => match state.resources.try_acquire(INFERENCE, owner, reason) {
            Ok(permit) => Ok(InferenceLease {
                _operation: guard,
                _permit: permit,
            }),
            Err(busy) => Err(busy),
        },
        Err(_) => Err(LeaseBusy {
            reason: state
                .resources
                .holder(INFERENCE)
                .map(|(_, reason)| reason)
                .unwrap_or_else(|| "another model operation".into()),
        }),
    }
}

/// Wait for the inference lease with the same wait semantics as `acquire`.
pub async fn acquire_inference(
    state: &crate::AppState,
    owner: &str,
    reason: &str,
    timeout: Option<Duration>,
    mut still_valid: impl FnMut() -> bool,
    mut on_wait: impl FnMut(&str),
) -> Result<InferenceLease, LeaseBusy> {
    let start = Instant::now();
    loop {
        match try_acquire_inference(state, owner, reason) {
            Ok(lease) => return Ok(lease),
            Err(busy) => {
                if timeout.is_some_and(|limit| start.elapsed() >= limit) {
                    return Err(busy);
                }
                if !still_valid() {
                    return Err(LeaseBusy {
                        reason: "cancelled".into(),
                    });
                }
                on_wait(&busy.reason);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_allows_one_holder_and_releases_on_drop() {
        let registry = std::sync::Arc::new(LeaseRegistry::default());
        let first = registry
            .try_acquire(BROWSER, "task-a", "browsing for task a")
            .unwrap();
        let busy = registry
            .try_acquire(BROWSER, "task-b", "browsing for task b")
            .unwrap_err();
        assert_eq!(busy.reason, "browsing for task a");
        drop(first);
        // Released on drop — the next task takes the tab.
        let _second = registry
            .try_acquire(BROWSER, "task-b", "browsing for task b")
            .unwrap();
        assert_eq!(registry.holder(BROWSER).unwrap().1, "browsing for task b");
    }

    #[tokio::test]
    async fn waiting_on_a_busy_slot_reports_the_holder_and_times_out() {
        let registry = std::sync::Arc::new(LeaseRegistry::default());
        let _first = registry
            .try_acquire(BROWSER, "task-a", "task a's page")
            .unwrap();
        let mut reported = String::new();
        let result = registry
            .acquire(
                BROWSER,
                "task-b",
                "task b's page",
                Some(Duration::from_millis(150)),
                || true,
                |reason| reported = reason.to_string(),
            )
            .await;
        assert!(result.is_err());
        assert!(reported.contains("task a"));
    }

    #[tokio::test]
    async fn acquire_succeeds_once_the_holder_drops() {
        let registry = std::sync::Arc::new(LeaseRegistry::default());
        let first = registry
            .try_acquire("workspace-write:/repo", "task-a", "writing")
            .unwrap();
        let other = std::sync::Arc::clone(&registry);
        let waiter = tokio::spawn(async move {
            other
                .acquire(
                    "workspace-write:/repo",
                    "task-b",
                    "writing",
                    Some(Duration::from_secs(5)),
                    || true,
                    |_| {},
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(first);
        waiter.await.unwrap().unwrap();
    }

    #[test]
    fn workspace_write_keys_normalize_paths() {
        assert_eq!(workspace_write("C:\\Repo\\"), workspace_write("c:/repo"));
    }

    #[test]
    fn background_allows_two_holders_not_three() {
        let registry = std::sync::Arc::new(LeaseRegistry::default());
        let _one = registry.try_acquire(BACKGROUND, "a", "job a").unwrap();
        let _two = registry.try_acquire(BACKGROUND, "b", "job b").unwrap();
        assert!(registry.try_acquire(BACKGROUND, "c", "job c").is_err());
        drop(_one);
        let _three = registry.try_acquire(BACKGROUND, "c", "job c").unwrap();
    }
}
