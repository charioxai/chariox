//! An agent asks its owner to let a new kernel join the owner's account.
//! The owner answers one card on any client; the owner's own enrolled kernel then approves the
//! new kernel's device login in Cloud, so no browser device page is needed.
use super::*;
use crate::session::{RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionChoiceStyle};
use sha2::Digest;

/// The owner answers within the code's own lifetime; the card never outlives it.
const MAX_WAIT: Duration = Duration::from_secs(600);

pub(super) const ENROLL_TOOL: &str = "chariox.kernels.enroll";

fn enrollment_error(code: &str, message: impl Into<String>) -> DaemonError {
    DaemonError::MachineRequest {
        code: code.into(),
        message: message.into(),
    }
}

pub(crate) fn kernel_enrollment_tool_specs() -> Vec<crate::transport::runtime_tools::RuntimeToolSpec>
{
    vec![crate::transport::runtime_tools::RuntimeToolSpec {
        name: ENROLL_TOOL.into(),
        description: "Ask the owner to let a new kernel you started join the owner's account. Pass user_code, the code the new kernel shows when it signs in (like ABCD-EFGH). The owner gets one card on every client they use and answers it there; no browser page and no sign-in as the owner is needed. Returns status approved when the owner approved and the kernel can finish joining by itself (its own sign-in then completes), or says plainly that the owner denied it or the code expired. Asking again with the same code never raises a second card.".into(),
        input_schema: serde_json::json!({"type":"object","properties":{"user_code":{"type":"string"}},"required":["user_code"],"additionalProperties":false}),
    }]
}

/// What Cloud says about the waiting login; the card is written from this, never from the agent's words.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Waiting {
    user_code: String,
    status: String,
    machine_name: Option<String>,
    expires_at: String,
}

fn cloud_call(
    profile: &crate::config::PersistedCloudRelayProfile,
    method: &str,
    path_and_query: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, DaemonError> {
    crate::owner_managed_bootstrap::admit_api_url(&profile.api_url)?;
    let credential = profile.kernel_credential.as_deref().unwrap_or_default();
    let request = crate::outbound_dial::agent_builder()
        .redirects(0)
        .timeout(Duration::from_secs(30))
        .build()
        .request(
            method,
            &format!("{}{path_and_query}", profile.api_url.trim_end_matches('/')),
        )
        .set("x-chariox-kernel-credential", credential);
    let sent = match body {
        Some(body) => request
            .set("content-type", "application/json")
            .send_string(&body.to_string()),
        None => request.call(),
    };
    match sent {
        Ok(response) => {
            use std::io::Read;
            let mut bytes = Vec::new();
            response
                .into_reader()
                .take(65537)
                .read_to_end(&mut bytes)
                .map_err(|_| enrollment_error("enrollment_unavailable", "Chariox did not answer; ask again in a moment"))?;
            serde_json::from_slice(&bytes)
                .map_err(|_| enrollment_error("enrollment_unavailable", "Chariox did not answer; ask again in a moment"))
        }
        Err(ureq::Error::Status(status, response)) => {
            let mut bytes = Vec::new();
            let message = {
                use std::io::Read;
                response.into_reader().take(8192).read_to_end(&mut bytes).ok().and_then(|_| {
                    serde_json::from_slice::<serde_json::Value>(&bytes)
                        .ok()
                        .and_then(|body| body["error"]["message"].as_str().map(str::to_owned))
                })
            };
            Err(match (status, message) {
                (404, _) => enrollment_error(
                    "enrollment_unknown",
                    "That code was not found; start a new login on the kernel and ask again",
                ),
                (401 | 403, _) => enrollment_error(
                    "enrollment_refused",
                    "Chariox did not accept this kernel's sign-in for that code; this kernel cannot approve it",
                ),
                (_, Some(message)) => enrollment_error("enrollment_refused", message),
                _ => enrollment_error("enrollment_unavailable", "Chariox did not answer; ask again in a moment"),
            })
        }
        Err(_) => Err(enrollment_error(
            "enrollment_unavailable",
            "Chariox did not answer; ask again in a moment",
        )),
    }
}

async fn blocking_call(
    profile: crate::config::PersistedCloudRelayProfile,
    method: &'static str,
    path_and_query: String,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, DaemonError> {
    tokio::task::spawn_blocking(move || cloud_call(&profile, method, &path_and_query, body))
        .await
        .map_err(|_| enrollment_error("enrollment_unavailable", "Chariox did not answer; ask again in a moment"))?
}

fn normalized_code(value: &str) -> Option<String> {
    let compact: String = value
        .trim()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (compact.len() == 8).then(|| format!("{}-{}", &compact[..4], &compact[4..]))
}

impl KernelRuntimeState {
    pub(crate) async fn kernel_enrollment_tool(
        &self,
        token: &str,
        args: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            user_code: String,
        }
        let args: Args = serde_json::from_value(args).map_err(|_| {
            enrollment_error("invalid_request", "Pass user_code, the code the new kernel shows")
        })?;
        let user_code = normalized_code(&args.user_code).ok_or_else(|| {
            enrollment_error("invalid_request", "That is not a kernel code; it looks like ABCD-EFGH")
        })?;
        let runs = self.owned.provider_store.get_runs_by_runtime_mcp_auth_token(token);
        let [run] = runs.as_slice() else {
            return Err(enrollment_error("not_allowed", "Enrolling a kernel needs one active agent"));
        };
        let reach = self
            .reach_for_id(&format!("{}{}", super::reach::REACH_PREFIX, run.id()))
            .ok_or_else(|| {
                enrollment_error(
                    "not_allowed",
                    "Only a running agent of the kernel owner may enroll a kernel",
                )
            })?;
        let room = self
            .owned
            .agent_store
            .get_agent(&reach.agent_id)
            .map_err(|_| enrollment_error("not_allowed", "This agent has no Room to ask the owner in"))?
            .session_id()
            .to_owned();
        let session = self.owned.session_store.get_session(&room)?;
        let config = self.owned.config_projection.snapshot();
        let profile = crate::runtime::cloud_relay_control::active_cloud_profile(&config)
            .filter(|p| p.kernel_credential.as_deref().is_some_and(|c| !c.is_empty()))
            .cloned()
            .ok_or_else(|| {
                enrollment_error(
                    "enrollment_unavailable",
                    "This kernel is not signed in to Chariox, so it cannot enroll another kernel",
                )
            })?;
        if profile.user_id != session.owner_user_id() {
            return Err(enrollment_error(
                "not_allowed",
                "Only the owner of this kernel can let another kernel join their account",
            ));
        }
        let query = format!(
            "/v1/kernel-device-approvals?userCode={}",
            crate::runtime::cloud_api_client::cloud_url_component(&user_code)
        );
        let waiting: Waiting =
            serde_json::from_value(blocking_call(profile.clone(), "GET", query, None).await?)
                .map_err(|_| enrollment_error("enrollment_unavailable", "Chariox did not answer; ask again in a moment"))?;
        let machine = waiting
            .machine_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
        match waiting.status.as_str() {
            "APPROVED" | "CONSUMED" => return Ok(done("approved", &machine)),
            "DENIED" => {
                return Err(enrollment_error(
                    "enrollment_denied",
                    "The owner denied this kernel, so it did not join; start a new login on the kernel to ask again",
                ))
            }
            "PENDING" => {}
            _ => return Err(expired()),
        }
        let id = format!(
            "kernel-enrollment-{:x}",
            sha2::Sha256::digest(format!("{}:{}", waiting.user_code, reach.agent_id).as_bytes())
        );
        let already_answered = !self
            .owned
            .durable_state_store
            .load_subject_events_by_kind(&id, "kernel.enrollment_approved", 1)?
            .is_empty();
        if !already_answered {
            let lifetime = chrono::DateTime::parse_from_rfc3339(&waiting.expires_at)
                .ok()
                .and_then(|at| (at.with_timezone(&chrono::Utc) - chrono::Utc::now()).to_std().ok())
                .ok_or_else(expired)?;
            let description = match &machine {
                Some(name) => format!("Let the new kernel {name} join your account? Its code is {}.", waiting.user_code),
                None => format!("Let a new kernel join your account? Its code is {}.", waiting.user_code),
            };
            let mut interaction = RuntimeInteraction::for_kernel_operation(
                &id,
                &id,
                "",
                description,
                vec![
                    RuntimeInteractionChoice::new("approve", "Approve", "approve", Some(RuntimeInteractionChoiceStyle::Primary)),
                    RuntimeInteractionChoice::new("deny", "Deny", "deny", Some(RuntimeInteractionChoiceStyle::Danger)),
                ],
            );
            interaction = interaction.with_related_agent_id(reach.agent_id.clone());
            let receiver = self
                .create_kernel_operation_interaction(&room, session.owner_user_id(), interaction)
                .await?;
            let mut pending = PendingEnrollment(Some((self.clone(), room.clone(), id.clone())));
            let answer = match tokio::time::timeout(lifetime.min(MAX_WAIT), receiver).await {
                Ok(Ok(answer)) => answer,
                _ => {
                    self.timeout_runtime_interaction(&room, &id).await?;
                    pending.0 = None;
                    return Err(enrollment_error(
                        "enrollment_expired",
                        "The owner did not answer before the code expired, so the kernel did not join; start a new login on the kernel and ask again",
                    ));
                }
            };
            pending.0 = None;
            if answer.status != "answered" || answer.choice_id.as_deref() != Some("approve") {
                let _ = blocking_call(
                    profile,
                    "POST",
                    "/v1/kernel-device-approvals".into(),
                    Some(serde_json::json!({"userCode": waiting.user_code, "decision": "deny"})),
                )
                .await;
                return Err(enrollment_error(
                    "enrollment_denied",
                    "The owner denied this kernel, so it did not join; start a new login on the kernel to ask again",
                ));
            }
            self.owned.durable_state_store.append_event(
                "kernel.enrollment_approved",
                Some(id),
                serde_json::json!({"owner": session.owner_user_id(), "approved_at_ms": crate::session::unix_epoch_ms()}),
            )?;
        }
        blocking_call(
            profile,
            "POST",
            "/v1/kernel-device-approvals".into(),
            Some(serde_json::json!({"userCode": waiting.user_code, "decision": "approve"})),
        )
        .await
        .map_err(|error| match error {
            DaemonError::MachineRequest { code, message } if code == "enrollment_refused" && message.contains("expired") => expired(),
            other => other,
        })?;
        Ok(done("approved", &machine))
    }
}

fn expired() -> DaemonError {
    enrollment_error(
        "enrollment_expired",
        "The code expired before the owner approved it, so the kernel did not join; start a new login on the kernel and ask again",
    )
}

fn done(status: &str, machine: &Option<String>) -> crate::transport::runtime_tools::RuntimeToolResult {
    crate::transport::runtime_tools::RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({"status": status, "kernel": machine}),
    }
}

/// Dropping a waiting request withdraws its card before an agent can leave a stale stripe.
struct PendingEnrollment(Option<(KernelRuntimeState, String, String)>);
impl Drop for PendingEnrollment {
    fn drop(&mut self) {
        if let Some((state, room, id)) = self.0.take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = state.timeout_runtime_interaction(&room, &id).await;
                });
            }
        }
    }
}

#[cfg(test)]
mod tests;
