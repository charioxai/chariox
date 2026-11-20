//! MP-08 / MP-10 / MP-11: reuse owner task consent at typed effect seams.
use super::*;
use crate::provider::{ProviderNativeInteractionResolution, RuntimeProviderRun};
use crate::session::{NativeInteractionOrigin, RuntimeInteractionKind};

impl KernelRuntimeState {
    /// Local admission uses the current run. Workers prove the exact leased
    /// prompt to the home on every use; a cached worker fence is not authority.
    pub(crate) async fn sudo_task_authorizes_run(&self, run: &RuntimeProviderRun) -> bool {
        match self
            .try_forward_leased_kernel_request(
                run,
                serde_json::json!({"request":{"ListSessions":null}}),
            )
            .await
        {
            Ok(Some(reply)) => reply.ok,
            Ok(None) => self.sudo_for_run(run.id()).is_ok(),
            Err(_) => false,
        }
    }

    /// Home-owned leased agents have a projected worker run rather than a
    /// local provider-store run. Resolve the same current home task there.
    pub(crate) async fn sudo_task_authorizes_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> bool {
        let Ok(agent) = self.owned.agent_store.get_agent(agent_id) else {
            return false;
        };
        if agent.session_id() != session_id {
            return false;
        }
        if let Some(remote) = agent.remote_execution() {
            return remote
                .active_worker_provider_run_id
                .as_deref()
                .is_some_and(|worker_run| {
                    let run = crate::provider::projected_leased_provider_run_id(
                        &remote.leased_agent_id,
                        worker_run,
                    );
                    self.sudo_for_run(&run).is_ok_and(|turn| {
                        turn.session_id == session_id && turn.agent_id == agent_id
                    })
                });
        }
        let Some(run) = self
            .owned
            .provider_store
            .get_run_for_agent(session_id, agent_id)
        else {
            return false;
        };
        self.sudo_task_authorizes_run(&run).await
    }

    /// Only provider permission-only gates can consume this authorization.
    /// Questions, secret input, identity choice and arbitrary critical decisions
    /// still use their ordinary interactions. Never persist a provider-wide grant.
    /// A worker-forwarded interaction names the raw worker run; the home task is
    /// bound to its projected leased run.
    pub(crate) fn sudo_native_permission_resolution(
        &self,
        session_id: &str,
        interaction: &RuntimeInteraction,
        forwarding: Option<&crate::transport::relay_peer::RemoteNativeInteractionContext>,
    ) -> Option<ProviderNativeInteractionResolution> {
        let origin_run = |origin: &NativeInteractionOrigin| match forwarding {
            Some(context) => crate::provider::projected_leased_provider_run_id(
                &context.leased_agent_id,
                &context.worker_provider_run_id,
            ),
            None => origin.provider_run_id().to_string(),
        };
        if interaction.kind() != RuntimeInteractionKind::Permission
            || ![
                "codex-command-approval-",
                "codex-file-change-approval-",
                "codex-exec-command-approval-",
                "codex-apply-patch-approval-",
                "opencode-permission-",
                "claude-permission-",
                "claude-native-permission-",
                "claude-rendered-permission-",
                "watched-process-",
                "app-view-action-",
            ]
            .iter()
            .any(|prefix| interaction.id().starts_with(prefix))
        {
            return None;
        }
        let origin = interaction.native_origin()?;
        let NativeInteractionOrigin::Prompt { prompt_id, .. } = origin else {
            return None;
        };
        let provider_run_id = origin_run(origin);
        let turn = self.sudo_for_run(&provider_run_id).ok()?;
        if turn.session_id != session_id
            || interaction.agent_id() != Some(turn.agent_id.as_str())
            || turn.prompt_id.as_deref() != Some(prompt_id.as_str())
            || !interaction
                .choices()
                .iter()
                .any(|choice| choice.id() == "allow_once" && !choice.requires_passkey())
        {
            return None;
        }
        crate::logging::info_with_fields(
            "runtime.sudo",
            "sudo task reused provider permission",
            serde_json::json!({"session_id":session_id,"agent_id":turn.agent_id,"provider_run_id":provider_run_id,"interaction_id":interaction.id()}),
        );
        Some(ProviderNativeInteractionResolution {
            status: "answered".into(),
            choice_id: Some("allow_once".into()),
            reply: None,
        })
    }
}
