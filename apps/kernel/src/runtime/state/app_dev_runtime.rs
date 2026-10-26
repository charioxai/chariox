//! A committed dev install automatically binds only agents of its own Room.
//! Normal grants retain their existing trust check, audit and tool refresh.
use super::*;
use crate::extension::{ExtensionGrant, ExtensionKind};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub(super) struct DevAppBindings {
    next_scan_ms: AtomicU64,
    bound: std::sync::Mutex<std::collections::BTreeSet<(String, String)>>,
}

impl KernelRuntimeState {
    pub(super) async fn sync_dev_app_bindings(&self) -> Result<(), DaemonError> {
        let state = &self.owned.dev_app_bindings;
        let now = crate::session::unix_epoch_ms();
        let next = state.next_scan_ms.load(Ordering::Relaxed);
        if now < next
            || state
                .next_scan_ms
                .compare_exchange(
                    next,
                    now.saturating_add(1000),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
        {
            return Ok(());
        }
        let store = self.owned.durable_state_store.clone();
        let installations =
            tokio::task::spawn_blocking(move || store.active_dev_app_installations())
                .await
                .map_err(|_| DaemonError::InternalInvariant {
                    operation: "apps.dev_bindings",
                    message: "Dev installation discovery did not complete".into(),
                })??;
        let agents = self.list_agents();
        for (owner, room, installation) in installations {
            for agent in &agents {
                if agent.owner_user_id() != owner || agent.session_id() != room {
                    continue;
                }
                let key = (agent.id().to_owned(), installation.clone());
                if state
                    .bound
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .contains(&key)
                {
                    continue;
                }
                if !agent.has_extension_grant(ExtensionKind::App, &installation) {
                    self.grant_agent_app(
                        agent.id(),
                        ExtensionGrant::new(ExtensionKind::App, &installation),
                        &owner,
                        None,
                    )
                    .await?;
                }
                state
                    .bound
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(key);
            }
        }
        Ok(())
    }
}
