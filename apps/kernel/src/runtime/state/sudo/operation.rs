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
        if interaction
            .id()
            .starts_with("claude-selected-workspace-trust-")
            && interaction.kind() == RuntimeInteractionKind::Permission
            && interaction
                .choices()
                .iter()
                .any(|choice| choice.id() == "allow_once" && !choice.requires_passkey())
            && matches!(
                interaction.native_origin(),
                Some(NativeInteractionOrigin::ProviderStartup { .. })
            )
            && interaction.native_origin().is_some_and(|origin| {
                self.sudo_authorized_workspace_startup(session_id, interaction, &origin_run(origin))
            })
        {
            return Some(ProviderNativeInteractionResolution {
                status: "answered".into(),
                choice_id: Some("allow_once".into()),
                reply: None,
            });
        }
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
    /// Trust the exact workspace of an already approved launch, even before
    /// its first prompt is sent. This grants no live sudo/tool authority.
    fn sudo_authorized_workspace_startup(
        &self,
        session_id: &str,
        interaction: &RuntimeInteraction,
        provider_run_id: &str,
    ) -> bool {
        let Some(agent_id) = interaction.agent_id() else {
            return false;
        };
        let Ok(agent) = self.owned.agent_store.get_agent(agent_id) else {
            return false;
        };
        let Ok(session) = self.owned.session_store.get_session(session_id) else {
            return false;
        };
        if agent.session_id() != session_id {
            return false;
        }
        if let Some(projected) = self.leased_agent_for_projected_run(provider_run_id) {
            if projected.id() != agent_id || projected.session_id() != session_id {
                return false;
            }
        } else {
            let Ok(run) = self.owned.provider_store.get_run(provider_run_id) else {
                return false;
            };
            if run.session_id() != session_id
                || run.agent_instance_id() != Some(agent_id)
                || self
                    .owned
                    .provider_store
                    .get_run_for_agent(session_id, agent_id)
                    .is_none_or(|current| current.id() != run.id())
                || run.state() != crate::provider::ProviderRunState::Running
            {
                return false;
            }
            let Some(workspace) = run
                .working_directory()
                .and_then(|path| path.canonicalize().ok())
            else {
                return false;
            };
            if Path::new(agent.worktree_id().unwrap_or_else(|| session.worktree_id()))
                .canonicalize()
                .ok()
                .as_ref()
                != Some(&workspace)
            {
                return false;
            }
        }
        let placement = agent
            .remote_execution()
            .map(|remote| remote.execution_lease_id.clone());
        let turns: Vec<_> = self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .values()
            .cloned()
            .collect();
        turns.iter().any(|turn| {
            turn.agent_id == agent_id
                && turn.session_id == session_id
                && turn.owner_user_id == session.owner_user_id()
                && turn.placement == placement
                && turn.deadline.is_some()
                && self.sudo_live(turn)
        })
    }
}
