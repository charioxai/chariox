//! MP-08 / MP-09 / MP-10 / MP-11: an outside agent asks the owner for a Vault value. The
//! prompt is a kernel-wide decision every owner terminal shows, and the owner can also answer
//! it on the link's page. The value goes from the answer into the Vault and nowhere else: the
//! agent gets the link and, later, `stored`, `cancelled` or `timed_out`.
use super::*;
use crate::local::{GetVaultSecretStatusRequest, RequestVaultSecretRequest, VaultSecretStatus};
use crate::runtime::kernel_access::{error, ACCESS_INTERACTION_SCOPE};

/// A refusal the agent reads: its code and the constant next step for it.
fn refusal(code: &str) -> DaemonError {
    DaemonError::MachineRequest {
        code: code.into(),
        message: crate::error::external_request_failure_message(code).into(),
    }
}

/// Operation subject of every secret request; the kernel-wide registration keys on it.
pub(crate) const VAULT_SECRET_OPERATION: &str = "secret-request:";
const SECRET_REQUEST_TIMEOUT_SEC: u64 = 600;
const LABEL_MAX_CHARS: usize = 80;
const HOSTS_MAX: usize = 8;
/// Finished requests stay readable this long, so a slow poll still learns the outcome.
const FINISHED_RETENTION_MS: u64 = 3_600_000;
const REQUESTS_KEPT: usize = 64;

/// What the requesting agent may read back about its request. Never the value.
#[derive(Debug, Clone)]
pub(crate) struct SecretRequest {
    grant_id: String,
    credential_id: String,
    status: VaultSecretStatus,
    expires_at_ms: u64,
}

impl KernelRuntimeState {
    pub(crate) async fn request_vault_secret(
        &self,
        grant_id: &str,
        request: RequestVaultSecretRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.authorize_external_request(
            grant_id,
            &LocalDaemonRequest::RequestVaultSecret(request.clone()),
        )?;
        let grant = self
            .access_grant_summary(grant_id)
            .ok_or_else(|| error("grant revoked or expired"))?;
        let snapshot = self.owned.config_projection.snapshot();
        if snapshot.user_config.credential_vault.agent_management
            == crate::config::CredentialVaultAgentManagementPolicy::Deny
        {
            return Err(refusal("secret_request_denied"));
        }
        let label = clean_label(&request.label)?;
        let hosts = clean_hosts(&request.hosts)?;
        let page = secret_page(&snapshot)?;
        self.require_unlocked_vault()?;
        let credential = vault_credential(&request.credential_id, &label, hosts, &grant)?;
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        let credential = replacing_or_new(&registry, credential, request.overwrite)?;

        let interaction_id = format!("secret-request-{:016x}", rand::random::<u64>());
        let who = super::super::kernel_access::requester::harness_name(grant.holder_harness)
            .unwrap_or("An external agent");
        let interaction = crate::session::RuntimeInteraction::for_vault_secret(
            &interaction_id,
            format!("{VAULT_SECRET_OPERATION}{interaction_id}"),
            "Value for the Vault",
            format!(
                "{who} asks for the value of {label}, for {}. It goes to your Chariox Vault and no agent sees it.",
                credential.allowed_hosts.join(", ")
            ),
            SECRET_REQUEST_TIMEOUT_SEC,
        );
        let expires_at = crate::session::unix_epoch_ms() + SECRET_REQUEST_TIMEOUT_SEC * 1000;
        let answer = self
            .create_kernel_operation_interaction(
                ACCESS_INTERACTION_SCOPE,
                &grant.owner_user_id,
                interaction,
            )
            .await?;
        self.record_secret_request(
            &interaction_id,
            SecretRequest {
                grant_id: grant_id.to_owned(),
                credential_id: credential.id.clone(),
                status: VaultSecretStatus::Pending,
                expires_at_ms: expires_at,
            },
        );
        let link = format!("{page}/s/{}/{interaction_id}", snapshot.daemon_id);
        let credential_id = credential.id.clone();
        let state = self.clone();
        let waiting_for = interaction_id.clone();
        tokio::spawn(async move {
            let status = state
                .store_answer(answer, credential, request.overwrite)
                .await;
            state.finish_secret_request(&waiting_for, status);
        });
        Ok(LocalDaemonResponse::VaultSecretRequested {
            interaction_id,
            link,
            expires_at,
            credential_id,
        })
    }

    pub(crate) fn vault_secret_status(
        &self,
        grant_id: &str,
        request: GetVaultSecretStatusRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.authorize_external_request(
            grant_id,
            &LocalDaemonRequest::GetVaultSecretStatus(request.clone()),
        )?;
        let access = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned");
        let known = access
            .secret_requests
            .get(&request.interaction_id)
            .filter(|known| known.grant_id == grant_id)
            .ok_or_else(|| refusal("secret_request_unknown"))?;
        Ok(LocalDaemonResponse::VaultSecretStatusReported {
            interaction_id: request.interaction_id,
            credential_id: known.credential_id.clone(),
            status: known.status,
        })
    }

    /// Waits for the owner's answer and stores it. The value lives in a `Zeroizing` string
    /// from the moment it leaves the answer until the Vault has it.
    pub(in crate::runtime::state) async fn store_answer(
        &self,
        answer: tokio::sync::oneshot::Receiver<PendingInteractionResolution>,
        credential: crate::config::UserCredentialConfig,
        overwrite: bool,
    ) -> VaultSecretStatus {
        let Ok(resolution) = answer.await else {
            return VaultSecretStatus::Cancelled;
        };
        if resolution.status == "timed_out" {
            return VaultSecretStatus::TimedOut;
        }
        let (Some("secret"), Some(value)) = (resolution.choice_id.as_deref(), resolution.reply)
        else {
            return VaultSecretStatus::Cancelled;
        };
        let value = zeroize::Zeroizing::new(value);
        let stored = async {
            let user_config = self.owned.config_projection.snapshot().user_config;
            let service = crate::secret::RuntimeSecretService::with_vault_config(
                crate::credential::load_user_credentials()?,
                &user_config.credential_vault,
            )?;
            let registry = crate::credential::CharioxCredentialRegistry::user()?;
            // The handle may have changed while the owner typed; store against what it is now.
            let credential = replacing_or_new(&registry, credential.clone(), overwrite)?;
            self.upsert_observed_vault_credential(
                &service,
                &registry,
                credential,
                value.as_str(),
                overwrite,
            )
            .await
        }
        .await;
        match stored {
            Ok(_) => VaultSecretStatus::Stored,
            Err(failure) => {
                crate::logging::error_with_fields(
                    "kernel.secret_request",
                    "the Vault refused the value the owner typed",
                    serde_json::json!({"credential_id": credential.id, "code": failure.to_string()}),
                );
                VaultSecretStatus::Cancelled
            }
        }
    }

    fn record_secret_request(&self, interaction_id: &str, request: SecretRequest) {
        let now = crate::session::unix_epoch_ms();
        let mut access = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned");
        access.secret_requests.retain(|_, known| {
            known.status == VaultSecretStatus::Pending
                || now < known.expires_at_ms.saturating_add(FINISHED_RETENTION_MS)
        });
        while access.secret_requests.len() >= REQUESTS_KEPT {
            let Some(oldest) = access
                .secret_requests
                .iter()
                .min_by_key(|(_, known)| known.expires_at_ms)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            access.secret_requests.remove(&oldest);
        }
        access
            .secret_requests
            .insert(interaction_id.to_owned(), request);
    }

    fn finish_secret_request(&self, interaction_id: &str, status: VaultSecretStatus) {
        if let Some(known) = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned")
            .secret_requests
            .get_mut(interaction_id)
        {
            known.status = status;
            known.expires_at_ms = known.expires_at_ms.min(crate::session::unix_epoch_ms());
        }
    }
}

/// The agent's words, shown after the kernel's own sentence: short, one line.
fn clean_label(label: &str) -> Result<String, DaemonError> {
    let label = label.trim();
    if label.is_empty()
        || label.chars().count() > LABEL_MAX_CHARS
        || label.chars().any(char::is_control)
    {
        return Err(refusal("secret_request_invalid"));
    }
    Ok(label.to_owned())
}

/// The hosts the key may go to, exactly as `allowed_hosts` matches them: `host` or
/// `host:port`, lowercase, no scheme, path or wildcard, and no IP literal but loopback.
fn clean_hosts(hosts: &[String]) -> Result<Vec<String>, DaemonError> {
    if hosts.is_empty() || hosts.len() > HOSTS_MAX {
        return Err(refusal("secret_request_invalid"));
    }
    let mut clean: Vec<String> = Vec::with_capacity(hosts.len());
    for entry in hosts {
        if !host_is_bindable(entry) {
            return Err(refusal("secret_request_invalid"));
        }
        if !clean.contains(entry) {
            clean.push(entry.clone());
        }
    }
    Ok(clean)
}

fn host_is_bindable(entry: &str) -> bool {
    // The port is split off first: the URL parser would drop a scheme's default port
    // (`:80`), and an entry that names it is valid.
    let (name, port) = match entry.rsplit_once(':') {
        Some((name, port))
            if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            (name, Some(port))
        }
        _ => (entry, None),
    };
    if port.is_some_and(|port| {
        port.parse::<u16>()
            .map_or(true, |number| number == 0 || number.to_string() != port)
    }) {
        return false;
    }
    let Ok(url) = url::Url::parse(&format!("http://{name}")) else {
        return false;
    };
    // Rebuilding the name from the parsed host rejects a scheme, path, credentials,
    // uppercase, wildcard and any spelling the matcher would not see as written.
    let Some(host) = url.host_str() else {
        return false;
    };
    let literal_ok = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(_)) => true,
        None => false,
    };
    host == name && url.port().is_none() && literal_ok && !entry.contains('*')
}

/// The page that serves the link: the configured one, else the enrolled Cloud's web app.
fn secret_page(snapshot: &crate::config::DaemonConfig) -> Result<String, DaemonError> {
    let configured = snapshot.user_config.secret_link.page_url.clone();
    let enrolled = snapshot.cloud_relay.as_ref().and_then(|profile| {
        url::Url::parse(&profile.api_url)
            .ok()
            .filter(|url| url.scheme() == "https")
            .map(|url| url.origin().ascii_serialization())
    });
    configured
        .or(enrolled)
        .map(|page| page.trim_end_matches('/').to_owned())
        .ok_or_else(|| refusal("secret_link_unavailable"))
}

/// The record a request stores its value under: a new handle as built, or, on `overwrite`,
/// the existing record untouched (its Vault key, injection and uses stay), provided the
/// request names exactly the hosts it is already bound to.
fn replacing_or_new(
    registry: &crate::credential::CharioxCredentialRegistry,
    requested: crate::config::UserCredentialConfig,
    overwrite: bool,
) -> Result<crate::config::UserCredentialConfig, DaemonError> {
    let Some(existing) = registry.get(&requested.id)? else {
        return Ok(requested);
    };
    if !overwrite {
        return Err(refusal("secret_request_exists"));
    }
    let bound = |hosts: &[String]| {
        let mut hosts = hosts.to_vec();
        hosts.sort();
        hosts
    };
    // Replacing the value must not send the old handle's key to new hosts.
    if !matches!(
        existing.source,
        crate::config::UserCredentialSourceConfig::Vault { .. }
    ) || bound(&existing.allowed_hosts) != bound(&requested.allowed_hosts)
    {
        return Err(refusal("secret_request_invalid"));
    }
    Ok(existing)
}

/// A new handle takes the plain Authorization header, bound to the hosts the agent named.
fn vault_credential(
    id: &str,
    label: &str,
    hosts: Vec<String>,
    grant: &crate::local::KernelAccessGrant,
) -> Result<crate::config::UserCredentialConfig, DaemonError> {
    let now = crate::session::unix_epoch_ms();
    let credential = crate::config::UserCredentialConfig {
        id: id.to_owned(),
        description: Some(label.to_owned()),
        source: crate::config::UserCredentialSourceConfig::Vault { key: id.to_owned() },
        allowed_hosts: hosts,
        allowed_uses: vec![crate::config::UserCredentialUse::Http],
        injection: crate::config::UserCredentialInjectionConfig::Header {
            name: "authorization".to_owned(),
            value: "Bearer ${secret}".to_owned(),
        },
        metadata: Some(crate::config::UserCredentialMetadataConfig {
            created_by_kind: Some("agent".to_owned()),
            created_by_id: None,
            session_id: None,
            provider: Some(crate::runtime::kernel_gateway::harness_name(grant)),
            provider_run_id: None,
            vault_key: Some(id.to_owned()),
            created_at_ms: Some(now),
            updated_at_ms: Some(now),
        }),
    };
    crate::credential::validate_credential_registration(&credential)
        .map_err(|_| refusal("secret_request_invalid"))?;
    Ok(credential)
}
