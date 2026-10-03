use super::*;

impl KernelRuntimeState {
    pub(super) fn enqueue_room_browser_manifest_sync_for_session(&self, session_id: &str) {
        let agent_ids = self
            .owned
            .agent_store
            .get_session_agents(session_id)
            .into_iter()
            .filter(|agent| agent.remote_execution().is_some())
            .map(|agent| agent.id().to_string())
            .collect::<Vec<_>>();
        if agent_ids.is_empty() {
            return;
        }
        let runtime = tokio::runtime::Handle::try_current().ok();
        for agent_id in agent_ids {
            let state = self.clone();
            let task = sync_room_browser_manifest_for_agent(state, agent_id.clone());
            if let Some(runtime) = runtime.as_ref() {
                runtime.spawn(task);
            } else {
                let worker_session_id = session_id.to_string();
                let worker_agent_id = agent_id.clone();
                let failure_agent_id = agent_id.clone();
                let spawn_result = std::thread::Builder::new()
                    .name("room-browser-manifest-sync".to_string())
                    .spawn(move || {
                        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                        else {
                            crate::logging::warn_with_fields(
                                "daemon.remote_extension",
                                "cannot start Room browser manifest refresh runtime",
                                serde_json::json!({
                                    "session_id": worker_session_id,
                                    "agent_id": worker_agent_id,
                                }),
                            );
                            return;
                        };
                        runtime.block_on(task);
                    });
                if let Err(error) = spawn_result {
                    crate::logging::warn_with_fields(
                        "daemon.remote_extension",
                        "cannot start Room browser manifest refresh thread",
                        serde_json::json!({
                            "session_id": session_id,
                            "agent_id": failure_agent_id,
                            "error": error.to_string(),
                        }),
                    );
                }
            }
        }
    }
}

async fn sync_room_browser_manifest_for_agent(state: KernelRuntimeState, agent_id: String) {
    let Ok(agent) = state.owned.agent_store.get_agent(&agent_id) else {
        return;
    };
    if let Err(error) = state
        .sync_remote_extension_manifest_for_agent(&agent, None, None)
        .await
    {
        crate::logging::warn_with_fields(
            "daemon.remote_extension",
            "Room browser manifest refresh failed",
            serde_json::json!({
                "agent_id": agent_id,
                "error": error.to_string(),
            }),
        );
    }
}
