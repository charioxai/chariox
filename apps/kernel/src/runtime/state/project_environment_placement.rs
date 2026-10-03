//! MP-08 / MP-10 / MP-11: Each lease imports once; prompts never become environment sync.
use crate::error::DaemonError;
use std::{collections::BTreeMap, future::Future, sync::Arc};
use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub(super) struct ProjectEnvironmentPlacements {
    leases: Arc<std::sync::Mutex<BTreeMap<String, Arc<Mutex<bool>>>>>,
}
impl ProjectEnvironmentPlacements {
    pub(super) async fn ensure<F, Fut>(&self, lease: &str, install: F) -> Result<(), DaemonError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), DaemonError>>,
    {
        let gate = {
            let mut leases = self.leases.lock().expect("Project environment placement");
            if !leases.contains_key(lease) && leases.len() >= 4096 {
                return Err(DaemonError::LocalTransport {
                    operation: "Project environment placement",
                    message: "Project environment placement capacity exceeded".into(),
                });
            }
            leases.entry(lease.into()).or_default().clone()
        };
        let mut installed = gate.lock().await;
        if !*installed {
            install().await?;
            *installed = true;
        }
        Ok(())
    }
    pub(super) fn retain(&self, active: &std::collections::BTreeSet<String>) {
        self.leases
            .lock()
            .expect("Project environment placement")
            .retain(|lease, _| active.contains(lease));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn mp08_mp10_prompts_reuse_successful_placement_and_retry_failure() {
        let store = ProjectEnvironmentPlacements::default();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let failure = || async {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(DaemonError::LocalTransport {
                operation: "fixture",
                message: "source unavailable".into(),
            })
        };
        assert!(store.ensure("lease", failure).await.is_err());
        for _ in 0..3 {
            store
                .ensure("lease", || async {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                })
                .await
                .unwrap();
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        store.retain(&Default::default());
        store
            .ensure("new-lease", || async {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }
}
