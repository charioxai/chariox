//! MP-08 / MP-11: same typed Computer contract for official MCP and terminals.
use super::kernel_browser_runtime::host_error;
use super::*;
use crate::local::KernelComputerCommand;
use crate::runtime::kernel_browser_host::KernelBrowserCapability;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
mod schema;
const LOADER: &str = "chariox.load_kernel_computer";
const TOOL: &str = "chariox.kernel_computer";

fn params(command: KernelComputerCommand) -> Result<serde_json::Value, DaemonError> {
    let mut value = serde_json::to_value(command)
        .map_err(|_| host_error("MP-08: invalid Computer command".into()))?;
    // MP-08 / MP-11: the shared helper sleeps 40 ms per character. Bound text
    // before any desktop action so valid input fits the existing 20 s RPC.
    if matches!(
        value["input"]["kind"].as_str(),
        Some("text" | "composition")
    ) && value["input"]["text"]
        .as_str()
        .is_some_and(|text| text.chars().count() > 128)
    {
        return Err(host_error(
            "MP-08: native text exceeds the execution budget; split into blocks of at most 128 characters".into(),
        ));
    }
    if let Some(target) = value.as_object_mut().and_then(|o| o.remove("target")) {
        for key in ["surface_id", "generation"] {
            let id = target[key]
                .as_str()
                .ok_or_else(|| host_error("MP-11: invalid desktop target".into()))?;
            if id.is_empty()
                || id.len() > 128
                || !id.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
            {
                return Err(host_error("MP-11: invalid desktop target".into()));
            }
            value[key] = id.into();
        }
    }
    if value["op"] == "launch_application"
        && value["application_id"].as_str().is_none_or(|id| {
            id.len() > 128
                || !id.ends_with(".desktop")
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        })
    {
        return Err(host_error("MP-11: invalid installed application ID".into()));
    }
    if value["query"]
        .as_str()
        .is_some_and(|q| q.is_empty() || q.len() > 4096)
    {
        return Err(host_error("MP-08: invalid OCR query".into()));
    }
    Ok(value)
}
impl KernelRuntimeState {
    pub(super) fn kernel_computer_tool_specs(
        &self,
        runs: &[crate::provider::RuntimeProviderRun],
    ) -> Vec<RuntimeToolSpec> {
        if !cfg!(target_os = "linux") {
            return Vec::new();
        }
        let [run] = runs else {
            return Vec::new();
        };
        let Some(agent) = self.kernel_computer_agent(run) else {
            return Vec::new();
        };
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let mut tools=vec![RuntimeToolSpec{name:LOADER.into(),description:"MP-08: load Computer tools for the user's kernel-owned Linux desktop. Focus grants new resources; existing tasks retain their explicit desktop until idle expiry or revoke. Room/slice membership grants no host access.".into(),input_schema:serde_json::json!({"type":"object","properties":{},"additionalProperties":false})}];
        if self.owned.kernel_browser_host.is_loaded_for(
            &user,
            agent.id(),
            KernelBrowserCapability::Computer,
        ) {
            tools.push(RuntimeToolSpec{name:TOOL.into(),description:"MP-08: control the owned host desktop via fresh surface/generation. snapshot returns scoped AT-SPI targets/tree_revision; stale targets require rediscovery. Input uses only op, target and input. Observer target_id/tree_revision belong only to target_action, never input. OCR and exact protected screenshots are on-demand; captures mask only currently plain fields filled by the kernel Vault; password dots remain visible. Agent input supports text/committed composition, key chords and bounded holds, left/middle/right click, drag, pointer holds, move, scroll and clipboard_write; use integer coordinates and click button 1=left, 2=middle or 3=right. Persistent keycode down/up require the human channel. Text, key and click require a proved native application destination; registered Vault targets refuse ordinary agent input. Native Chrome controls can invoke owner-only extensions without opening a page, so use document-bound Browser tools for Chrome input. Use installed_applications with the current target, then launch_application with one returned application_id. No command strings, environment or free arguments. The kernel owns the application process and cleanup. Human takeover pauses agent mutations. Never use video timing as observation.".into(),input_schema:schema::tool_schema()});
        }
        tools
    }
    pub(super) async fn kernel_computer_terminal_request(
        &self,
        caller: &crate::runtime::command::KernelCommand,
        command: KernelComputerCommand,
    ) -> Result<serde_json::Value, DaemonError> {
        if !cfg!(target_os = "linux") {
            return Err(host_error(
                "MP-08: owned Computer desktop currently requires Linux".into(),
            ));
        }
        let (user, actor) = self.kernel_browser_terminal_context(caller)?;
        self.refresh_user_domain_grants();
        let mut parameters = params(command)?;
        parameters["observed_by"] = actor.into();
        let admission = self
            .owned
            .kernel_browser_host
            .admit_terminal(&user, caller.terminal_lifetime.clone().unwrap_or_default());
        self.kernel_browser_operation_admitted(&user, Some(admission), "host.computer", parameters)
            .await
    }
    pub(super) async fn try_kernel_computer_tool(
        &self,
        token: &str,
        name: &str,
        arguments: serde_json::Value,
    ) -> Option<Result<RuntimeToolResult, DaemonError>> {
        if ![LOADER, TOOL].contains(&name) {
            return None;
        }
        Some(self.kernel_computer_tool(token, name, arguments).await)
    }
    async fn kernel_computer_tool(
        &self,
        token: &str,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(host_error(
                "MP-11: admitted local provider run required".into(),
            ));
        };
        let leased_grant = self.leased_computer_tool_authority(run).await?;
        let agent = if leased_grant.is_some() {
            self.kernel_computer_agent(run)
                .ok_or_else(|| host_error("MP-11: not_granted: Computer grant revoked".into()))?
        } else {
            self.user_domain_tool_agent(run).await?
        };
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let host = self.owned.kernel_browser_host.clone();
        if name == LOADER {
            if arguments.as_object().is_none_or(|o| !o.is_empty()) {
                return Err(host_error(
                    "MP-08: Computer loader accepts no arguments".into(),
                ));
            }
            if leased_grant.is_none() {
                host.load_for(&user, agent.id(), KernelBrowserCapability::Computer)
                    .map_err(host_error)?;
            }
            return Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({"loaded":TOOL}),
            });
        }
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            command: KernelComputerCommand,
        }
        let request: Args = serde_json::from_value(arguments)
            .map_err(|_| host_error("MP-08: invalid Computer command; input accepts only op, target and input. Use target_id/tree_revision only with target_action. Reload Computer tools for the current operation schema.".into()))?;
        if matches!(
            request.command,
            KernelComputerCommand::Takeover { .. }
                | KernelComputerCommand::Release { .. }
                | KernelComputerCommand::Actors { .. }
                | KernelComputerCommand::DisplaySubscribe { .. }
        ) {
            return Err(host_error("MP-11: human desktop channel required".into()));
        }
        let authority = self.clone();
        let auth_token = token.to_string();
        let run_id = run.id().to_string();
        let expected_grant = leased_grant.clone();
        let authority_agent = agent.id().to_string();
        let admission = host
            .admit_for(&user, agent.id(), KernelBrowserCapability::Computer)
            .map_err(host_error)?
            .with_agent_label(agent.alias())
            .with_authority(move || {
                let current = authority
                    .owned
                    .provider_store
                    .get_runs_by_runtime_mcp_auth_token(&auth_token);
                let [run] = current.as_slice() else {
                    return false;
                };
                run.id() == run_id
                    && authority.kernel_computer_agent(run).is_some()
                    && expected_grant.as_ref().is_none_or(|grant| {
                        authority
                            .owned
                            .leased_computer_grants
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .get(&authority_agent)
                            == Some(grant)
                    })
            });
        let mut parameters = params(request.command)?;
        if let Some(grant) = leased_grant.as_ref() {
            if parameters["op"] == "start" {
                parameters["op"] = "state".into();
            }
            if parameters["op"] != "state"
                && (parameters["surface_id"].as_str() != Some(&grant.target.surface_id)
                    || parameters["generation"].as_str() != Some(&grant.target.generation))
            {
                return Err(host_error(
                    "MP-11: not_granted: Computer target differs from the explicit desktop grant"
                        .into(),
                ));
            }
        }
        let mut payload = self
            .kernel_browser_operation_admitted(&user, Some(admission), "host.computer", parameters)
            .await?;
        if let Some(grant) = leased_grant {
            if payload["surface_id"].is_string()
                && (payload["surface_id"].as_str() != Some(&grant.target.surface_id)
                    || payload["generation"].as_str() != Some(&grant.target.generation))
            {
                return Err(host_error(
                    "MP-11: stale native surface or generation".into(),
                ));
            }
        }
        // MCP receives one native image content block, with no base64 text copy.
        if let Some(data) = payload
            .as_object_mut()
            .and_then(|o| o.remove("data_base64"))
        {
            payload["image_base64"] = data;
        }
        Ok(RuntimeToolResult { ok: true, payload })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mp11_launch_only_accepts_an_installed_desktop_id() {
        let target = crate::local::KernelDesktopTarget {
            surface_id: "s".into(),
            generation: "g".into(),
        };
        for id in [
            "/bin/sh",
            "../editor.desktop",
            "editor.desktop;env",
            "x\n.desktop",
        ] {
            assert!(params(KernelComputerCommand::LaunchApplication {
                target: target.clone(),
                application_id: id.into()
            })
            .is_err());
        }
        assert!(params(KernelComputerCommand::LaunchApplication {
            target,
            application_id: "org.xfce.mousepad.desktop".into()
        })
        .is_ok());
    }
    #[test]
    fn mp08_native_text_budget_is_checked_before_backend_dispatch() {
        for kind in ["text", "composition"] {
            let command = |length| {
                serde_json::from_value::<KernelComputerCommand>(serde_json::json!({
                    "op":"input", "target":{"surface_id":"s", "generation":"g"},
                    "input":{"kind":kind,"text":"😀".repeat(length)}
                }))
                .unwrap()
            };
            assert!(params(command(128)).is_ok());
            assert!(params(command(129)).is_err());
            assert!(params(command(600)).is_err());
        }
    }
    #[test]
    fn mp11_computer_target_refuses_empty_identity_and_actor_forgery() {
        assert!(params(KernelComputerCommand::Snapshot {
            target: crate::local::KernelDesktopTarget {
                surface_id: "".into(),
                generation: "g".into()
            }
        })
        .is_err());
        assert!(serde_json::from_value::<KernelComputerCommand>(
            serde_json::json!({"op":"state","observed_by":"forged"})
        )
        .is_err());
    }
}
