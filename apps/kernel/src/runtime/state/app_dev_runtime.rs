//! A committed dev install automatically binds only agents of its own Room.
//! Normal grants retain their existing trust check, audit and tool refresh.
use super::*;
use crate::extension::ExtensionKind;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub(super) struct DevAppBindings {
    next_scan_ms: AtomicU64,
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
        let mut failure = None;
        for (owner, room, installation) in installations {
            for agent in &agents {
                if agent.owner_user_id() != owner || agent.session_id() != room {
                    continue;
                }
                let result = async {
                    let current = self.owned.agent_store.get_agent(agent.id())?;
                    if current.has_extension_grant(ExtensionKind::App, &installation) {
                        return Ok(());
                    }
                    if let Some(_bound) = self.grant_room_dev_app(&current, &installation).await? {
                        #[cfg(any(
                            target_os = "macos",
                            all(target_os = "linux", target_env = "gnu")
                        ))]
                        self.refresh_agent_runtime_tool_catalog(_bound.session_id(), _bound.id())
                            .await?;
                    }
                    Ok::<_, DaemonError>(())
                }
                .await;
                if let Err(error) = result {
                    failure.get_or_insert(error);
                }
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}
