//! Cloud-only owner authorization. Capabilities never enter IPC, packages or disk.
use super::outbound_service::{ManagedContextTransferTarget, ManagedContextTransferTicket};
use super::owner_managed::{admission_error, cloud_admission_error};
use crate::config::{DaemonConfig, PersistedCloudRelayProfile};
use crate::error::DaemonError;
use serde::Deserialize;
use serde_json::{json, Value};
use zeroize::Zeroize;

pub(crate) const TICKET_ENDPOINT: &str = "/v1/managed-kernels/context/ticket";
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryPeer {
    target_id: String,
    peer: ManagedContextTransferTarget,
}
#[derive(Deserialize)]
struct Directory {
    peers: Vec<DirectoryPeer>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Binding {
    kind: String,
    source_target_id: String,
    source: ManagedContextTransferTarget,
    target: ManagedContextTransferTarget,
    context_selection: Value,
}
#[derive(Deserialize)]
struct Issued {
    #[serde(flatten)]
    binding: Binding,
    ticket: String,
}

fn cloud_authorization_error(retryable: bool) -> DaemonError {
    DaemonError::ManagedContext {
        code: "owner_managed_context_unauthorized",
        operation: "reauthorize owner-managed transfer",
        message: if retryable {
            "Cloud owner authorization is temporarily unavailable"
        } else {
            "Cloud refused owner-managed source/target authorization"
        }
        .into(),
        retryable,
    }
}

async fn kernel_cloud_request<T: serde::de::DeserializeOwned + Send + 'static>(
    profile: &PersistedCloudRelayProfile,
    path: &'static str,
    body: Option<Value>,
) -> Result<T, DaemonError> {
    let profile = profile.clone();
    tokio::task::spawn_blocking(move || {
        let credential = profile
            .kernel_credential
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| admission_error("Kernel enrollment credential is required"))?;
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(15))
            .redirects(0)
            .build();
        let url = format!("{}{}", profile.api_url.trim_end_matches('/'), path);
        let response = if let Some(body) = body {
            agent
                .post(&url)
                .set("x-chariox-kernel-credential", credential)
                .set("content-type", "application/json")
                .send_string(&body.to_string())
        } else {
            agent
                .get(&url)
                .set("x-chariox-kernel-credential", credential)
                .call()
        }
        .map_err(|error| {
            cloud_authorization_error(match error {
                ureq::Error::Transport(_) => true,
                ureq::Error::Status(status, _) => status == 408 || status == 429 || status >= 500,
            })
        })?;
        // Bounded, typed response; raw API errors never appear in runtime notices.
        let reader = std::io::Read::take(response.into_reader(), 256 * 1024);
        serde_json::from_reader(reader).map_err(|_| cloud_authorization_error(true))
    })
    .await
    .map_err(|_| cloud_authorization_error(true))?
}

fn source_peer(config: &DaemonConfig) -> Result<ManagedContextTransferTarget, DaemonError> {
    let profile = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| admission_error("Source kernel has no enrolled owner"))?;
    Ok(ManagedContextTransferTarget {
        relay_realm_id: profile.realm_id.clone(),
        machine_id: profile
            .machine_id
            .clone()
            .ok_or_else(|| admission_error("Source machine identity is missing"))?,
        kernel_id: config.daemon_id.clone(),
        relay_public_key: config.relay_public_key.clone(),
        key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
            &config.relay_public_key,
        ),
    })
}

pub(crate) async fn authorize_export(
    config: &DaemonConfig,
    requested: &ManagedContextTransferTicket,
    allow_first_issuance: bool,
) -> Result<ManagedContextTransferTicket, DaemonError> {
    let profile = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| admission_error("Source kernel has no enrolled owner"))?;
    let source = source_peer(config)?;
    let plan_binding = requested.context_plan.package_binding();
    // Retries re-check the consumed operation using the source credential;
    // the capability itself is still consumed exactly once.
    let stored: Result<Binding, _> = crate::runtime::cloud_api_client::post_cloud_json(
        profile.api_url.clone(),
        TICKET_ENDPOINT,
        json!({
            "kernelCredential": profile.kernel_credential, "ownerManagedExport": {
                "contextId": plan_binding.context_id, "planDigest": plan_binding.plan_digest,
                "source": source, "target": requested.target,
            },
        }),
    )
    .await;
    let serialized_plan = serde_json::to_value(&requested.context_plan)
        .map_err(|_| admission_error("Invalid owner context plan"))?;
    let selected = json!({ "kernelContext": serialized_plan["kernelContext"], "developmentSetup": serialized_plan["developmentSetup"] });
    match stored {
        Ok(binding) => {
            if binding.kind != "owner_managed_machine"
                || binding.source != source
                || binding.target != requested.target
                || binding.context_selection != selected
            {
                return Err(admission_error("Cloud owner authorization pins changed"));
            }
            return Ok(requested.clone());
        }
        Err(error) => {
            // Never issue another capability for a resumed context ID: Cloud
            // retains consumed rows, including expired ones, under a unique ID.
            let may_issue = allow_first_issuance
                && crate::runtime::cloud_api_client::cloud_error_code(&error)
                    == Some("authorization_expired");
            if !may_issue {
                return Err(cloud_admission_error(error));
            }
        }
    }
    let directory: Directory =
        kernel_cloud_request(profile, "/v1/owner-managed-context-tickets/peers", None).await?;
    let peer = directory
        .peers
        .into_iter()
        .find(|peer| peer.peer == source)
        .ok_or_else(|| admission_error("Source kernel is offline or not the same user"))?;
    let plan = serde_json::to_value(&requested.context_plan)
        .map_err(|_| admission_error("Invalid owner context plan"))?;
    let selection = json!({ "kernelContext": plan["kernelContext"], "developmentSetup": plan["developmentSetup"] });
    let mut issued: Issued = kernel_cloud_request(profile, "/v1/owner-managed-context-tickets", Some(json!({
        "sourceTargetId": peer.target_id, "target": requested.target, "contextSelection": selection, "ttl_seconds": 300,
    }))).await?;
    let expected = |binding: &Binding| {
        binding.kind == "owner_managed_machine"
            && binding.source_target_id == peer.target_id
            && binding.source == source
            && binding.target == requested.target
            && binding.context_selection == selection
    };
    if !expected(&issued.binding) {
        issued.ticket.zeroize();
        return Err(admission_error("Cloud owner authorization pins changed"));
    }
    let body = json!({ "kernelCredential": profile.kernel_credential, "ownerManaged": {
        "ticket": issued.ticket, "source": source, "target": requested.target, "contextSelection": selection,
        "contextId": plan["contextId"], "planDigest": plan["planDigest"],
    } });
    issued.ticket.zeroize();
    let consumed: Binding = crate::runtime::cloud_api_client::post_cloud_json(
        profile.api_url.clone(),
        TICKET_ENDPOINT,
        body,
    )
    .await
    .map_err(cloud_admission_error)?;
    if !expected(&consumed) {
        return Err(admission_error("Cloud owner authorization pins changed"));
    }
    Ok(requested.clone())
}

pub(crate) async fn authorize_import(
    config: &DaemonConfig,
    caller: &chariox_relay::protocol::RelayCallerIdentity,
    source_kernel_id: &str,
    plan: &super::package::ManagedContextPlanBinding,
) -> Result<(), DaemonError> {
    let profile = config
        .cloud_relay
        .as_ref()
        .ok_or_else(|| admission_error("Target kernel has no enrolled owner"))?;
    let target = source_peer(config)?;
    let directory: Directory =
        kernel_cloud_request(profile, "/v1/owner-managed-context-tickets/peers", None).await?;
    let source = directory
        .peers
        .into_iter()
        .find(|peer| {
            peer.peer.kernel_id == source_kernel_id
                && peer.peer.relay_realm_id == caller.realm_id
                && Some(peer.peer.key_thumbprint.as_str())
                    == caller.public_key_thumbprint.as_deref()
        })
        .ok_or_else(|| admission_error("Authenticated source is offline or not the same user"))?;
    let binding: Binding = crate::runtime::cloud_api_client::post_cloud_json(profile.api_url.clone(), TICKET_ENDPOINT, json!({
        "kernelCredential": profile.kernel_credential, "ownerManagedImport": {
            "contextId": plan.context_id, "planDigest": plan.plan_digest, "source": source.peer, "target": target,
        },
    })).await.map_err(cloud_admission_error)?;
    if binding.kind != "owner_managed_machine"
        || binding.source != source.peer
        || binding.target != target
        || binding.source_target_id != source.target_id
    {
        return Err(admission_error(
            "Cloud owner authorization does not match authenticated encrypted peers",
        ));
    }
    let development = match &plan.development {
        super::package::ManagedContextDevelopmentSelection::Empty => json!({ "kind": "empty" }),
        super::package::ManagedContextDevelopmentSelection::SourceProject {
            project_id,
            repositories,
        } => json!({
            "kind": "source_project", "projectId": project_id,
            "repositories": repositories.iter().map(|repository| json!({ "role": repository.role,
                "workspaceId": repository.workspace_id, "worktreeId": repository.worktree_id })).collect::<Vec<_>>()
        }),
    };
    if binding.context_selection
        != json!({ "kernelContext": plan.kernel_context, "developmentSetup": development })
    {
        return Err(admission_error(
            "Cloud owner authorization selection changed",
        ));
    }
    // Cloud's consumed context/plan binding fixes the selection; the target still
    // verifies the credential-free plan and package through the shared importer.
    Ok(())
}

#[cfg(test)]
#[path = "owner_authority_tests.rs"]
mod tests;
