//! Store the credential through the existing Vault lifecycle/authority boundary.
use super::*;
use sha2::{Digest, Sha256};

#[derive(Clone, Default)]
pub(in crate::runtime::state) struct RegistrationClaims(Arc<std::sync::Mutex<BTreeSet<String>>>);

pub(super) struct RegistrationClaim {
    claims: RegistrationClaims,
    handle: String,
}
impl RegistrationClaims {
    pub(super) fn claim(&self, handle: &str) -> Result<RegistrationClaim, DaemonError> {
        if !self
            .0
            .lock()
            .map_err(|_| error("auth.md registration store is unavailable"))?
            .insert(handle.into())
        {
            return Err(error(
                "this auth.md request is already running; wait for its result before retrying",
            ));
        }
        Ok(RegistrationClaim {
            claims: self.clone(),
            handle: handle.into(),
        })
    }
}
impl Drop for RegistrationClaim {
    fn drop(&mut self) {
        if let Ok(mut claims) = self.claims.0.lock() {
            claims.remove(&self.handle);
        }
    }
}

pub(super) fn handle(turn: &RegistrationActor, request_id: &str) -> String {
    let digest = Sha256::digest(
        [
            turn.owner_user_id.as_str(),
            &turn.session_id,
            &turn.agent_id,
            request_id.trim(),
        ]
        .join("\n"),
    );
    format!("auth-{}", &format!("{digest:x}")[..24])
}

pub(super) fn committed(
    existing: &UserCredentialConfig,
    authority: &Authority,
    label: &str,
) -> Result<RuntimeToolResult, DaemonError> {
    let metadata = existing.metadata.as_ref();
    if metadata.and_then(|m| m.created_by_kind.as_deref()) != Some("agent_auth")
        || metadata.and_then(|m| m.created_by_id.as_deref())
            != Some(authority.actor.agent_id.as_str())
        || metadata.and_then(|m| m.session_id.as_deref())
            != Some(authority.actor.session_id.as_str())
        || existing.allowed_hosts != [authority.resource.origin().ascii_serialization()]
        || existing.description.as_deref() != Some(label)
    {
        return Err(error(
            "request_id already names another auth.md credential; use a new request_id",
        ));
    }
    authority.check()?;
    Ok(RuntimeToolResult {
        ok: true,
        payload: json!({"credential_id": existing.id, "label": label}),
    })
}

impl KernelRuntimeState {
    pub(super) async fn store_agent_auth_credential(
        &self,
        authority: &Authority,
        registry: &crate::credential::CharioxCredentialRegistry,
        handle: &str,
        label: &str,
        issued: IssuedCredential,
    ) -> Result<RuntimeToolResult, DaemonError> {
        authority.check()?;
        let now = crate::session::unix_epoch_ms();
        let credential = UserCredentialConfig {
            id: handle.into(),
            description: Some(label.into()),
            source: crate::config::UserCredentialSourceConfig::Vault { key: handle.into() },
            allowed_hosts: vec![http::endpoint(&issued.resource)?
                .origin()
                .ascii_serialization()],
            allowed_uses: vec![crate::config::UserCredentialUse::Http],
            injection: crate::config::UserCredentialInjectionConfig::Header {
                name: "authorization".into(),
                value: "Bearer ${secret}".into(),
            },
            metadata: Some(UserCredentialMetadataConfig {
                created_by_kind: Some("agent_auth".into()),
                created_by_id: Some(authority.actor.agent_id.clone()),
                session_id: Some(authority.actor.session_id.clone()),
                provider: None,
                provider_run_id: authority.actor.provider_run_id.clone(),
                vault_key: Some(handle.into()),
                created_at_ms: Some(now),
                updated_at_ms: Some(now),
            }),
        };
        authority
            .authorized_state
            .upsert_observed_vault_credential(
                &self.home_runtime_secret_service()?,
                registry,
                credential.clone(),
                issued.value.as_str(),
                false,
            )
            .await?;
        record_registration(self, &credential, &issued.registration_id)?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: json!({"credential_id": handle, "label": label}),
        })
    }
}

/// S7's integration seam. Credential provenance is already durable in the Vault
/// registry; when the resource register is present it records this account here.
fn record_registration(
    _state: &KernelRuntimeState,
    _credential: &UserCredentialConfig,
    _registration_id: &str,
) -> Result<(), DaemonError> {
    Ok(())
}
