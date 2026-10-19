//! MP-08 / MP-10 / MP-11: host Vault input uses the owned desktop, not a Room.
use super::kernel_browser_runtime::host_error;
use super::*;
use crate::runtime::kernel_browser_host::{KernelBrowserCapability, KernelBrowserHost};
use crate::transport::room_browser_controller::{
    RoomBrowserControllerCommand, RoomComputerInputAction, RoomComputerSecretInput,
    RoomComputerSecretTarget,
};
use crate::transport::runtime_tools::RuntimeToolResult;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeField {
    pid: u32,
    started: String,
    path: String,
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HostSecretTarget {
    surface_id: String,
    generation: String,
    target: RoomComputerSecretTarget,
    field: NativeField,
}
impl KernelRuntimeState {
    pub(super) async fn dispatch_host_computer_secret_input(
        &self,
        session: &str,
        agent_id: &str,
        credential: &str,
    ) -> Result<RuntimeToolResult, DaemonError> {
        use super::kernel_browser_secret_runtime::require_vault_owner;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session {
            return Err(DaemonError::AgentNotInSession {
                session_id: session.into(),
                agent_id: agent_id.into(),
            });
        }
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        require_vault_owner(&user)?;
        let admission = self
            .owned
            .kernel_browser_host
            .admit_for(&user, agent_id, KernelBrowserCapability::Computer)
            .map_err(host_error)?;
        let run = self
            .owned
            .provider_store
            .get_run_for_agent(session, agent_id)
            .ok_or_else(|| host_error("MP-11: not_granted: active provider run required".into()))?;
        let run_id = run.id().to_owned();
        let state = self.clone();
        let session_id = session.to_owned();
        let authority_agent = agent_id.to_owned();
        let owner = agent.owner_user_id().to_owned();
        let admission = admission
            .with_agent_label(agent.alias())
            .with_authority(move || {
                state.authorize_current_forwarded_binding().is_ok()
                    && state
                        .owned
                        .provider_store
                        .get_run(&run_id)
                        .is_ok_and(|run| state.kernel_computer_agent(&run).is_some())
                    && state
                        .owned
                        .agent_store
                        .get_agent(&authority_agent)
                        .is_ok_and(|agent| {
                            agent.session_id() == session_id && agent.owner_user_id() == owner
                        })
                    && state
                        .owned
                        .session_snapshot(&session_id)
                        .is_ok_and(|session| {
                            session.has_member(&owner)
                                && session.status() != crate::session::SessionStatus::Ended
                        })
            });
        let check = || {
            self.owned
                .kernel_browser_host
                .check_admission(Some(&admission))
                .map_err(host_error)
        };
        check()?;
        let protection = &self.owned.kernel_browser_secret_observations;
        let scope = KernelBrowserHost::profile_key(&user);
        let epoch = protection.revision(&scope)?;
        let target = self.host_computer_secret_target(&user, &admission).await?;
        let approval = self
            .ensure_computer_login_approved(
                session,
                agent_id,
                credential,
                &format!(
                    "host:{}:{}:{}:{}:{}",
                    target.surface_id,
                    target.generation,
                    target.field.pid,
                    target.field.started,
                    target.target.active_window
                ),
                &format!(
                    "native window {}, AT-SPI process {} (start {}), field {}",
                    target.target.active_window,
                    target.field.pid,
                    target.field.started,
                    target.field.path
                ),
            )
            .await?;
        let authority = approval.authority();
        let already_unlocked = approval.has_unlock();
        let state = self.clone();
        let admission =
            admission.with_authority(move || state.computer_secret_approval_live(&approval));
        let check = || {
            self.owned
                .kernel_browser_host
                .check_admission(Some(&admission))
                .map_err(host_error)
        };
        let _unlock = if already_unlocked {
            None
        } else {
            Some(
                self.ensure_vault_unlocked_for_agent(
                    session,
                    agent_id,
                    "runtime_tool_paste_secret_to_computer",
                )
                .await?,
            )
        };
        check()?;
        let (service, _vault_guard) = self
            .scoped_secret_input_service(protection, &scope, credential)
            .await?;
        let _barrier = protection.barrier(&scope)?.write_owned().await;
        check()?;
        if protection.revision(&scope)? != epoch
            || self
                .host_computer_secret_target_bound(&user, &admission)
                .await?
                != target
        {
            return Err(host_error(
                "computer credential input aborted: focused field, window or protection changed"
                    .into(),
            ));
        }
        // Reloaded metadata and native identity are authoritative before value resolution.
        service.validate_computer_secret_input(credential)?;
        let secret = self
            .with_authorized_app_side_effect(|_| {
                self.with_forwarded_binding_operation(|| {
                    Ok(zeroize::Zeroizing::new(
                        service.computer_secret_input(credential)?,
                    ))
                })
            })
            .await?;
        check()?;
        let command = RoomBrowserControllerCommand::ComputerInput {
            action_id: "host-computer-secret".into(),
            actor_id: agent_id.into(),
            runtime_generation: 0,
            viewport_revision: 0,
            desktop_pixel_width: 0,
            desktop_pixel_height: 0,
            action: RoomComputerInputAction::SecretText {
                input: RoomComputerSecretInput::from_zeroizing(secret),
                expected_target: target.target.clone(),
            },
        };
        // Register before physical input so partial/failed fills stay protected.
        protection.register_command(&scope, &command)?;
        let RoomBrowserControllerCommand::ComputerInput {
            action: RoomComputerInputAction::SecretText { input, .. },
            ..
        } = command
        else {
            unreachable!()
        };
        let mut params = serde_json::to_value(&target)
            .map_err(|_| host_error("invalid native target".into()))?;
        params["op"] = "secret_input".into();
        params["text"] = Value::String(input.into_zeroizing().to_string());
        let _input = self.owned.kernel_browser_host.display_gate(&user).input();
        check()?;
        self.kernel_browser_bound_operation(&user, Some(&admission), "host.computer", params, true)
            .await?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: json!({"submitted":true,"permission_authority":authority,"credential_id":credential,"target":"desktop_focus","filled_field":{"window":target.target.active_window,"pid":target.field.pid,"started":target.field.started,"path":target.field.path}}),
        })
    }

    async fn host_computer_secret_target(
        &self,
        user: &str,
        admission: &crate::runtime::kernel_browser_host::KernelBrowserAdmission,
    ) -> Result<HostSecretTarget, DaemonError> {
        let scope = KernelBrowserHost::profile_key(user);
        let _barrier = self
            .owned
            .kernel_browser_secret_observations
            .barrier(&scope)?
            .read_owned()
            .await;
        self.host_computer_secret_target_bound(user, admission)
            .await
    }
    async fn host_computer_secret_target_bound(
        &self,
        user: &str,
        admission: &crate::runtime::kernel_browser_host::KernelBrowserAdmission,
    ) -> Result<HostSecretTarget, DaemonError> {
        let state = self
            .kernel_browser_bound_operation(
                user,
                Some(admission),
                "host.computer",
                json!({"op":"state"}),
                true,
            )
            .await?;
        let result = self.kernel_browser_bound_operation(user, Some(admission), "host.computer", json!({"op":"secret_target","surface_id":state["surface_id"],"generation":state["generation"]}), true).await?;
        let target: HostSecretTarget = serde_json::from_value(result)
            .map_err(|_| host_error("focused native field is unavailable".into()))?;
        if !target.target.valid()
            || target.surface_id.is_empty()
            || target.generation.is_empty()
            || target.field.pid <= 1
            || target.field.started.is_empty()
            || target.field.path.is_empty()
        {
            return Err(host_error("focused native field is unavailable".into()));
        }
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mp11_private_vault_operations_are_not_public_computer_commands() {
        for op in ["secret_input", "secret_target"] {
            assert!(
                serde_json::from_value::<crate::local::KernelComputerCommand>(
                    serde_json::json!({"op":op,"surface_id":"s","generation":"g"})
                )
                .is_err()
            );
        }
    }
}
