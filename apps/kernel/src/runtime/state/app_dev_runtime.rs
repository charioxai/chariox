//! A committed dev install automatically binds only agents of its own Room.
//! Normal grants retain their existing trust check, audit and tool refresh.
use super::*;
use crate::extension::ExtensionKind;

#[derive(Default)]
pub(super) struct DevAppBindings {
    next_scan_ms: tokio::sync::Mutex<u64>,
}

impl KernelRuntimeState {
    pub(super) async fn sync_dev_app_bindings(&self) -> Result<(), DaemonError> {
        let state = &self.owned.dev_app_bindings;
        // Keep the deadline locked for the full scan: a slow pass cannot
        // overlap the next maintenance tick and race its own grant attempts.
        let Ok(mut next_scan_ms) = state.next_scan_ms.try_lock() else {
            return Ok(());
        };
        let now = crate::session::unix_epoch_ms();
        if now < *next_scan_ms {
            return Ok(());
        }
        *next_scan_ms = now.saturating_add(1000);
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
