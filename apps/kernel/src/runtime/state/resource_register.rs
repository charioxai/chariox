//! MP-08 / MP-09 / MP-10 / MP-11 AU-11: project-scoped inventory of external resources.
use super::KernelRuntimeState;
use crate::error::DaemonError;
use crate::local::{CreatedResource, CreatedResourceState, KernelSudoTurn, ResourceRevokeMethod};
use crate::runtime::kernel_access::error;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

mod automatic;
mod revoke;

pub(super) const RECORD: &str = "chariox.mandate.record_resource";
pub(super) const LIST: &str = "chariox.mandate.list_resources";
pub(super) const SET_REVOKE: &str = "chariox.mandate.set_resource_revoke";
pub(super) const REVOKE: &str = "chariox.mandate.revoke_resource";

pub(super) fn specs() -> Vec<RuntimeToolSpec> {
    let text = || json!({"type":"string","minLength":1,"maxLength":512});
    let revoke = json!({"oneOf":[
        {"type":"object","required":["kind","request"],"properties":{"kind":{"const":"http"},"request":{
            "type":"object","required":["credential_id","url","method"],"properties":{
                "credential_id":text(),"url":text(),"method":{"type":"string","enum":["POST","DELETE"]},
                "headers":{"type":"object","additionalProperties":{"type":"string"}},
                "body_text":{"type":"string"},"body_json":{},"timeout_ms":{"type":"integer"},
                "max_response_bytes":{"type":"integer"}},"additionalProperties":false}},"additionalProperties":false},
        {"type":"object","required":["kind","url"],"properties":{"kind":{"const":"url"},"url":text()},"additionalProperties":false},
        {"type":"object","required":["kind","reason"],"properties":{"kind":{"const":"unavailable"},"reason":text()},"additionalProperties":false}
    ]});
    vec![
        RuntimeToolSpec { name: RECORD.into(), description: "Record an external resource you created in this Room's project. Requires elevation. Metadata only: reference Vault handles, never secret values. request_id makes retries idempotent. Store an HTTP revoke request using a Vault credential, an owner settings URL, or the reason no revoke method is available.".into(),
            input_schema: json!({"type":"object","required":["request_id","service","kind","name","revoke"],"properties":{
                "request_id":text(),"service":text(),"kind":text(),"name":text(),"url":text(),"credential_id":text(),"revoke":revoke.clone()},"additionalProperties":false}) },
        RuntimeToolSpec { name: SET_REVOKE.into(), description: "Set the revoke method on an existing resource, including automatic Vault or SSH entries. Requires elevation in the resource project. Metadata and Vault handles only; cannot change provenance or a revocation in progress.".into(),
            input_schema: json!({"type":"object","required":["resource_id","revoke"],"properties":{"resource_id":text(),"revoke":revoke},"additionalProperties":false}) },
        RuntimeToolSpec { name: LIST.into(), description: "List resources created for this Room's project, including Vault handle links, provenance and revoke methods. Requires elevation. Returns metadata only.".into(),
            input_schema: json!({"type":"object","properties":{},"additionalProperties":false}) },
        RuntimeToolSpec { name: REVOKE.into(), description: "Revoke a registered resource using its stored HTTP request and Vault credential. Requires elevation. Returns only status, never HTTP response content or credentials. For an owner URL, return the URL for the owner to open; the row remains active until a service revocation is confirmed. After a crash, unfinished revocations become revoke_failed; check the service before retrying.".into(),
            input_schema: json!({"type":"object","required":["resource_id"],"properties":{"resource_id":text()},"additionalProperties":false}) },
    ]
}

#[derive(Clone, PartialEq, Eq)]
struct Authority {
    authorization_id: String,
    owner_user_id: String,
    session_id: String,
    project_id: String,
    agent_id: String,
    prompt_id: String,
    provider_run_id: String,
}

impl Authority {
    fn same_turn(&self, other: &Self) -> bool {
        self == other
    }
}

impl KernelRuntimeState {
    /// S1 integration seam: replace the live-sudo admission here with mandate-or-sudo.
    /// Resolve project and provenance in the kernel; arguments never choose another project.
    fn resource_register_authority(
        &self,
        run_id: &str,
        _service: &str,
        _kind: &str,
    ) -> Result<Authority, DaemonError> {
        self.authorize_current_external_command()?;
        let turn = self
            .sudo_for_provider_run(run_id)
            .map_err(|_| error("resource register requires a live sudo window"))?;
        let session = self.owned.session_store.get_session(&turn.session_id)?;
        Ok(Authority {
            authorization_id: turn.entry_id,
            owner_user_id: turn.owner_user_id,
            session_id: turn.session_id,
            project_id: session.project_id().into(),
            agent_id: turn.agent_id,
            prompt_id: turn
                .prompt_id
                .ok_or_else(|| error("resource register requires a running prompt"))?,
            provider_run_id: turn
                .provider_run_id
                .ok_or_else(|| error("resource register requires a running provider"))?,
        })
    }

    pub(super) async fn resource_register_tool(
        &self,
        token: &str,
        name: &str,
        args: Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(error("resource register requires one active provider run"));
        };
        if let Some(result) = self
            .try_dispatch_remote_meta_runtime_tool_call(run, name, args.clone())
            .await?
        {
            return Ok(result);
        }
        self.resource_register_for_run(run.id(), name, args).await
    }

    pub(in crate::runtime::state) async fn resource_register_for_run(
        &self,
        run_id: &str,
        name: &str,
        args: Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let payload = match name {
            RECORD => {
                let args: RecordArgs =
                    serde_json::from_value(args).map_err(|_| error("invalid resource metadata"))?;
                let authority =
                    self.resource_register_authority(run_id, &args.service, &args.kind)?;
                args.validate(self)?;
                let id = resource_id(&[
                    &authority.owner_user_id,
                    &authority.project_id,
                    &authority.agent_id,
                    &args.request_id,
                ]);
                let row = CreatedResource {
                    id,
                    project_id: authority.project_id,
                    service: args.service,
                    kind: args.kind,
                    name: args.name,
                    url: args.url,
                    credential_id: args.credential_id,
                    created_by: authority.agent_id,
                    created_at_ms: crate::session::unix_epoch_ms(),
                    revoke: args.revoke,
                    state: CreatedResourceState::Active,
                    revoked_at_ms: None,
                    last_http_status: None,
                };
                let row = self
                    .owned
                    .durable_state_store
                    .record_created_resource(&authority.owner_user_id, &row)?;
                json!({"resource":row})
            }
            LIST => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Empty {}
                let _: Empty = serde_json::from_value(args)
                    .map_err(|_| error("list_resources takes no arguments"))?;
                let authority = self.resource_register_authority(run_id, "", "list")?;
                json!({"resources":self.owned.durable_state_store.created_resources(&authority.owner_user_id, &authority.project_id)?})
            }
            SET_REVOKE => self.set_resource_revoke(run_id, args)?,
            REVOKE => self.revoke_created_resource(run_id, args).await?,
            _ => return Err(error("unknown resource register tool")),
        };
        let ok = !matches!(
            payload.get("status").and_then(Value::as_str),
            Some("revoke_failed" | "revoke_unavailable")
        );
        Ok(RuntimeToolResult { ok, payload })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordArgs {
    request_id: String,
    service: String,
    kind: String,
    name: String,
    url: Option<String>,
    credential_id: Option<String>,
    revoke: ResourceRevokeMethod,
}

impl RecordArgs {
    fn validate(&self, state: &KernelRuntimeState) -> Result<(), DaemonError> {
        for value in [&self.request_id, &self.service, &self.kind, &self.name] {
            bounded(value)?;
        }
        if let Some(url) = &self.url {
            safe_url(url)?;
        }
        if let Some(id) = &self.credential_id {
            bounded(id)?;
            if crate::credential::CharioxCredentialRegistry::user()?
                .get(id)?
                .is_none()
            {
                return Err(error(
                    "credential_id is not registered in the owner's Vault",
                ));
            }
        }
        match &self.revoke {
            ResourceRevokeMethod::Http { request } => {
                safe_url(&request.url)?;
                if !matches!(request.method.as_str(), "DELETE" | "POST") {
                    return Err(error("HTTP revocation requires method DELETE or POST"));
                }
                if request.body_text.is_some() && request.body_json.is_some() {
                    return Err(error(
                        "choose either body_text or body_json for the revoke request",
                    ));
                }
                if request.max_response_bytes == 0 {
                    return Err(error("max_response_bytes must be positive"));
                }
                if serde_json::to_vec(request)
                    .map_err(|_| error("invalid revoke request"))?
                    .len()
                    > 16384
                    || request.timeout_ms == 0
                    || request.timeout_ms > 30000
                {
                    return Err(error(
                        "revoke request must fit 16 KiB and timeout_ms must be 1-30000",
                    ));
                }
                state
                    .home_runtime_secret_service()?
                    .validate_http_request_target(&request.credential_id, &request.url)?;
            }
            ResourceRevokeMethod::Url { url } => {
                safe_url(url)?;
            }
            ResourceRevokeMethod::Unavailable { reason } => bounded(reason)?,
        }
        Ok(())
    }
}

fn bounded(value: &str) -> Result<(), DaemonError> {
    if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(error(
            "resource metadata fields must be 1-512 characters without control characters",
        ));
    }
    Ok(())
}

fn safe_url(raw: &str) -> Result<(), DaemonError> {
    bounded(raw)?;
    let url = url::Url::parse(raw).map_err(|_| error("resource URL is invalid"))?;
    let host = url.host_str().unwrap_or_default();
    let loopback = host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if host.is_empty()
        || !url.username().is_empty()
        || url.password().is_some()
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
    {
        return Err(error(
            "resource URLs require HTTPS (or loopback HTTP) without embedded credentials",
        ));
    }
    Ok(())
}

fn resource_id(parts: &[&str]) -> String {
    // JSON preserves field boundaries even if a caller includes separators in metadata.
    let digest = Sha256::digest(serde_json::to_vec(parts).expect("string tuple encodes"));
    format!("resource-{}", &format!("{digest:x}")[..32])
}
