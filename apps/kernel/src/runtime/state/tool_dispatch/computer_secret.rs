use crate::error::DaemonError;

use super::*;

impl KernelRuntimeState {
    pub(super) async fn dispatch_computer_secret_input_tool(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        agent_id: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.dispatch_computer_secret_input_for_authority(
            provider_run.session_id(),
            agent_id,
            arguments,
        )
        .await
    }

    pub(super) async fn dispatch_forwarded_home_computer_secret_input_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        agent: &crate::agent::AgentInstance,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.dispatch_computer_secret_input_for_authority(
            &context.home_session_id,
            agent.id(),
            arguments,
        )
        .await
    }

    async fn dispatch_computer_secret_input_for_authority(
        &self,
        session_id: &str,
        agent_id: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let args = serde_json::from_value::<
            crate::transport::runtime_tools::PasteSecretToComputerArgs,
        >(arguments)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "runtime_tool_paste_secret_to_computer",
            message: format!("invalid tool arguments: {error}"),
        })?;
        self.home_runtime_secret_service()?
            .validate_computer_secret_input(&args.credential_id)?;
        // A host desktop has no Room environment. Never fall back after a
        // Room failure: slices keep their home-owned generation and approval.
        match self.room_environment_snapshot(session_id) {
            Err(crate::session::EnvironmentError::EnvironmentNotFound { .. })
                if self.slice_kernel_id().is_none() =>
            {
                return self
                    .dispatch_host_computer_secret_input(session_id, agent_id, &args.credential_id)
                    .await;
            }
            _ => {}
        }
        let approved_generation = self
            .reconcile_room_environment_actors(session_id, None)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "computer_secret_input.target",
                message: error.code().to_string(),
            })?
            .runtime_generation;
        let target = self
            .computer_secret_input_target(session_id, agent_id)
            .await?;
        let (app, description) = target.login_scope(approved_generation);
        let approval = self
            .ensure_computer_login_approved(session_id, agent_id, &args.credential_id, &app, &description)
            .await?;
        let _vault_unlock = if approval.has_unlock() {
            None
        } else {
            Some(
                self.ensure_vault_unlocked_for_agent(
                    session_id,
                    agent_id,
                    "runtime_tool_paste_secret_to_computer",
                )
                .await?,
            )
        };
        // Unlock may itself wait for the user. Reject a changed target before
        // resolving the credential, then check again in the physical helper.
        if self
            .computer_secret_input_target(session_id, agent_id)
            .await?
            != target
        {
            return Err(DaemonError::LocalTransport {
                operation: "computer_secret_input.target",
                message: "computer credential input aborted: focused control or window changed"
                    .into(),
            });
        }
        let approval = std::sync::Arc::new(approval);
        let check_approval = || {
            if !self.computer_secret_approval_live(&approval) {
                return Err(DaemonError::LocalTransport {
                    operation: "computer_secret_input.approval",
                    message: "login permission expired or was revoked".into(),
                });
            }
            Ok(())
        };
        check_approval()?;
        let (service, _vault_observation_guard) = self
            .room_secret_input_service(session_id, &args.credential_id)
            .await?;
        let secret = self
            .with_authorized_app_side_effect(|_| {
                self.with_forwarded_binding_operation(|| {
                    Ok(zeroize::Zeroizing::new(
                        service.computer_secret_input(&args.credential_id)?,
                    ))
                })
            })
            .await?;
        self.authorize_current_forwarded_binding()?;
        check_approval()?;
        let live_state = self.clone();
        let live_approval = approval.clone();
        let live = move || live_state.computer_secret_approval_live(&live_approval);
        let bound = if let Some(turn) = &approval.sudo_turn {
            let authority =
                crate::local::LocalDaemonRequest::ListSessions(crate::local::ListSessionsRequest);
            self.with_external_command_authority(Some((&turn.entry_id, &authority)))
                .with_sudo_binding(turn.prompt_id.clone().zip(turn.provider_run_id.clone()))
        } else {
            self.clone()
        }
        .with_computer_secret_live(live);
        let execution = bound
            .execute_computer_input_as_agent_for_generation(
                session_id,
                agent_id,
                crate::transport::room_browser_controller::RoomComputerInputAction::SecretText {
                    input:
                        crate::transport::room_browser_controller::RoomComputerSecretInput::from_zeroizing(
                            secret,
                        ),
                    expected_target: target,
                },
                Some(approved_generation),
            )
            .await?;
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "submitted": true,
                "permission_authority": approval.authority(),
                "credential_id": args.credential_id,
                "target": "desktop_focus",
                "action_id": execution.action_id,
                "actor_id": execution.actor_id,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn mp08_mp11_host_secret_reaches_computer_admission_without_room() {
        use crate::runtime::state::browser_controller_action_execution_runtime_state::computer_input_reconcile_test_support::TestRoom;
        let _lock = crate::env_lock::lock();
        let root =
            std::env::temp_dir().join(format!("cufill-host-admission-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        struct Restore(Option<std::ffi::OsString>, std::path::PathBuf);
        impl Drop for Restore {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.1);
                match &self.0 {
                    Some(value) => std::env::set_var("CHARIOX_HOME", value),
                    None => std::env::remove_var("CHARIOX_HOME"),
                }
            }
        }
        let _restore = Restore(std::env::var_os("CHARIOX_HOME"), root.clone());
        std::env::set_var("CHARIOX_HOME", root.join("home"));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let fixture = TestRoom::new("host-vault-no-room");
        let (session, agent) = {
            let mut app = fixture.runtime.app.lock().await;
            let (session, _) = crate::app::KernelSessionService::new(&mut app)
                .create_session(crate::session::CreateSessionRequest::new(
                    workspace.to_str().unwrap(),
                    workspace.to_str().unwrap(),
                ))
                .unwrap();
            let agent = crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(crate::agent::CreateAgentRequest::new(
                    session.id(),
                    "dev-stub",
                ))
                .unwrap();
            (session, agent)
        };
        crate::credential::CharioxCredentialRegistry::user()
            .unwrap()
            .upsert(crate::config::UserCredentialConfig {
                id: "native-login".into(),
                description: None,
                source: crate::config::UserCredentialSourceConfig::Vault {
                    key: "not-resolved".into(),
                },
                allowed_hosts: vec![],
                allowed_uses: vec![crate::config::UserCredentialUse::Computer],
                injection: crate::config::UserCredentialInjectionConfig::Computer,
                metadata: None,
            })
            .unwrap();
        assert!(matches!(
            fixture.runtime.room_environment_snapshot(session.id()),
            Err(crate::session::EnvironmentError::EnvironmentNotFound { .. })
        ));
        let error = fixture
            .runtime
            .dispatch_computer_secret_input_for_authority(
                session.id(),
                agent.id(),
                serde_json::json!({"credential_id":"native-login"}),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, crate::error::DaemonError::UserDomainRefused { .. }),
            "host input must reach Computer admission, without a Room or Vault read: {error}"
        );
        assert!(fixture
            .runtime
            .owned
            .session_snapshot(session.id())
            .unwrap()
            .active_interactions()
            .is_empty());
    }
}
