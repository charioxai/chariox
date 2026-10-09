//! MP-08 / MP-11: same typed Computer contract for official MCP and terminals.
use super::kernel_browser_runtime::host_error;
use super::*;
use crate::local::KernelComputerCommand;
use crate::runtime::kernel_browser_host::KernelBrowserCapability;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
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
            if id.is_empty() || id.len() > 128 {
                return Err(host_error("MP-11: invalid desktop target".into()));
            }
            value[key] = id.into();
        }
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
        let Some(agent) = self.kernel_browser_agent(run) else {
            return Vec::new();
        };
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let mut tools=vec![RuntimeToolSpec{name:LOADER.into(),description:"MP-08: load Computer tools for the user's kernel-owned Linux desktop. Focus grants new resources; existing tasks retain their explicit desktop until idle expiry or revoke. Room/slice membership grants no host access.".into(),input_schema:serde_json::json!({"type":"object","properties":{},"additionalProperties":false})}];
        if self.owned.kernel_browser_host.is_loaded_for(
            &user,
            agent.id(),
            KernelBrowserCapability::Computer,
        ) {
            tools.push(RuntimeToolSpec{name:TOOL.into(),description:"MP-08: control the owned host desktop via fresh surface/generation. snapshot returns scoped AT-SPI targets/tree_revision; stale targets require rediscovery. OCR and exact protected screenshots are on-demand; captures mask only currently plain fields filled by the kernel Vault; password dots remain visible. Input uses text/committed composition, key chords, bounded holds, mouse and clipboard; human takeover pauses agent input. Persistent keycode down/up is a human channel. Never use video timing as observation.".into(),input_schema:serde_json::json!({"type":"object","properties":{"command":{"type":"object","properties":{"op":{"enum":["start","state","snapshot","screenshot","ocr","clipboard_read","input","target_action"]},"target":{"type":"object","properties":{"surface_id":{"type":"string","minLength":1,"maxLength":128},"generation":{"type":"string","minLength":1,"maxLength":128}},"required":["surface_id","generation"],"additionalProperties":false},"query":{"type":["string","null"],"maxLength":4096},"input":{"type":"object","properties":{"kind":{"enum":["text","composition","key","hold","pointer_hold","click","move","drag","scroll","clipboard_write"]},"text":{"type":"string","maxLength":65536,"description":"text/composition: at most 128 Unicode characters per action; clipboard_write: at most 65536 UTF-8 bytes"},"key":{"type":"string","maxLength":128},"duration_ms":{"type":"integer","minimum":1,"maximum":10000},"x":{"type":"integer","minimum":0,"maximum":1279},"y":{"type":"integer","minimum":0,"maximum":799},"to_x":{"type":"integer","minimum":0,"maximum":1279},"to_y":{"type":"integer","minimum":0,"maximum":799},"button":{"type":"integer","minimum":1,"maximum":3},"steps":{"type":"integer","minimum":-100,"maximum":100}},"required":["kind"],"additionalProperties":false},"tree_revision":{"type":"integer","minimum":1},"target_id":{"type":"string","maxLength":128},"action":{"type":"string","maxLength":128}},"required":["op"],"additionalProperties":false}},"required":["command"],"additionalProperties":false})});
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
        let agent = self.user_domain_tool_agent(run).await?;
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let host = self.owned.kernel_browser_host.clone();
        if name == LOADER {
            if arguments.as_object().is_none_or(|o| !o.is_empty()) {
                return Err(host_error(
                    "MP-08: Computer loader accepts no arguments".into(),
                ));
            }
            host.load_for(&user, agent.id(), KernelBrowserCapability::Computer)
                .map_err(host_error)?;
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
            .map_err(|_| host_error("MP-08: invalid Computer command".into()))?;
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
        let admission = host
            .admit_for(&user, agent.id(), KernelBrowserCapability::Computer)
            .map_err(host_error)?
            .with_authority(move || {
                let current = authority
                    .owned
                    .provider_store
                    .get_runs_by_runtime_mcp_auth_token(&auth_token);
                let [run] = current.as_slice() else {
                    return false;
                };
                run.id() == run_id && authority.kernel_browser_agent(run).is_some()
            });
        let mut payload = self
            .kernel_browser_operation_admitted(
                &user,
                Some(admission),
                "host.computer",
                params(request.command)?,
            )
            .await?;
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
