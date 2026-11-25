//! `chariox.vault.copy_for_use`: a Vault entry made for one use gets a second
//! entry for another declared use (an `https://api.github.com` call from an
//! entry made for `git push`), without the value leaving the kernel. The owner
//! types or sends the secret once; each use is its own entry with its own
//! destination.
use super::*;
use crate::config::{
    UserCredentialConfig, UserCredentialInjectionConfig, UserCredentialMetadataConfig,
    UserCredentialSourceConfig, UserCredentialUse, VaultUseTarget,
};
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
use sha2::{Digest, Sha256};

pub(crate) const VAULT_COPY_FOR_USE: &str = "chariox.vault.copy_for_use";
const COPIED_KIND: &str = "vault_copy";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CopyArgs {
    request_id: String,
    from: String,
    use_binding: crate::config::VaultUseBinding,
    #[serde(default)]
    label: Option<String>,
}

pub(crate) fn copy_for_use_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: VAULT_COPY_FOR_USE.into(),
        description: "Copy a Vault entry's value into a NEW entry for one other declared use, inside the Vault: the model sees only the new credential_id, never the value. `from` is the credential_id to copy. use_binding declares the one destination of the copy: a site origin (an HTTPS API; the entry then sends `Authorization: Bearer <value>` to exactly that origin, for chariox.http_request_with_credential and App accounts), a command environment name and origin, a command stdin origin, or an absolute workspace file path. Payment cards, TOTP seeds and backup codes cannot be copied. Retrying with the same request_id returns the same credential.".into(),
        input_schema: serde_json::json!({"type":"object","required":["request_id","from","use_binding"],"properties":{
            "request_id":{"type":"string","minLength":1,"maxLength":128},
            "from":{"type":"string","minLength":1,"maxLength":128,"description":"credential_id or unique description of the entry to copy"},
            "use_binding": copy_binding_schema(),
            "label":{"type":"string","maxLength":200}},
            "additionalProperties":false}),
    }
}

fn copy_binding_schema() -> serde_json::Value {
    let mut schema = capture_binding_schema();
    schema["properties"]["target"]["oneOf"]
        .as_array_mut()
        .expect("capture binding lists its targets")
        .push(serde_json::json!({"type":"object","required":["kind","origin"],"additionalProperties":false,"properties":{
            "kind":{"const":"site"},"origin":{"type":"string","description":"An https origin such as https://api.github.com"}}}));
    schema
}

/// Arguments the Vault tool and the home kernel both refuse the same way.
pub(super) fn parse_copy_args(arguments: serde_json::Value) -> Result<CopyArgs, DaemonError> {
    let args: CopyArgs = serde_json::from_value(arguments)
        .map_err(|e| error(format!("invalid Vault copy arguments: {e}")))?;
    if args.request_id.trim().is_empty() || args.request_id.len() > 128 {
        return Err(error("request_id must be 1-128 characters"));
    }
    if args.label.as_ref().is_some_and(|label| label.len() > 200) {
        return Err(error("label must be at most 200 characters"));
    }
    Ok(args)
}

/// Checks a destination against the workspace it names.
type BindingCheck<'a> =
    &'a (dyn Fn(&crate::config::VaultUseBinding) -> Result<(), DaemonError> + Sync);

impl KernelRuntimeState {
    pub(crate) async fn vault_copy_for_use(
        &self,
        token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let context = self.vault_leased_context(token).await?;
        self.recheck_vault_command_context(&context)?;
        let args = parse_copy_args(arguments.clone())?;
        if let Some(home) = context.home.clone() {
            // The destination is a place in this workspace; the home keeps the Vault.
            validate_copy_binding(&context.run, &args.use_binding)?;
            return self
                .forward_vault_tool_to_home(home, VAULT_COPY_FOR_USE, arguments)
                .await;
        }
        // The kernel owns settlement and commit even if the MCP waiter goes away.
        let state = self.clone();
        tokio::spawn(async move {
            state
                .copy_in_vault(
                    &context.actor(),
                    &|| state.recheck_vault_command_context(&context),
                    &|binding| validate_copy_binding(&context.run, binding),
                    args,
                )
                .await
        })
        .await
        .map_err(|_| error("the copy runner stopped unexpectedly"))?
    }

    /// Copies inside the Vault of this kernel: the owner's own, or the home's for an agent on
    /// a leased worker, whose destination the worker already checked.
    pub(super) async fn copy_in_vault(
        &self,
        actor: &VaultActor,
        recheck: VaultRecheck<'_>,
        check_binding: BindingCheck<'_>,
        mut args: CopyArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        args.from = self
            .resolve_entry_refs([args.from.as_str()])?
            .pop()
            .unwrap_or_default();
        args.use_binding.target.validate()?;
        if let VaultUseTarget::Site { origin } = &args.use_binding.target {
            if !origin.starts_with("https://") {
                return Err(error("a site copy needs an https origin"));
            }
        }
        let handle = copied_handle(actor, &args.request_id);
        let _claim = self.owned.vault_capture_claims.claim(&handle).await?;
        recheck()?;
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        if let Some(existing) = registry.get(&handle)? {
            return committed(&existing, actor, &args.from, &args.use_binding);
        }
        check_binding(&args.use_binding)?;
        let source = registry
            .get(&args.from)?
            .ok_or_else(|| error("no Vault entry has this credential_id"))?;
        let UserCredentialSourceConfig::Vault { key } = &source.source else {
            return Err(error("this entry does not store its value in the Vault"));
        };
        if source
            .metadata
            .as_ref()
            .is_some_and(|metadata| metadata.payment_card.is_some())
            || matches!(
                source.injection,
                UserCredentialInjectionConfig::Totp(_) | UserCredentialInjectionConfig::BackupCodes
            )
        {
            return Err(error(
                "payment cards, TOTP seeds and backup codes cannot be copied",
            ));
        }
        let _unlock = self
            .ensure_vault_unlocked_for_agent(&actor.session_id, &actor.agent_id, "vault_copy")
            .await?;
        recheck()?;
        let value = self
            .home_runtime_secret_service()?
            .vault_secret_for_dictation(key)?
            .ok_or_else(|| error("this entry holds no value"))?;
        let now = crate::session::unix_epoch_ms();
        let (allowed_host, allowed_use, injection) = match &args.use_binding.target {
            VaultUseTarget::Site { origin } => site_bearer_use(origin)?,
            VaultUseTarget::File { .. } => (
                source.allowed_hosts.first().cloned().unwrap_or_default(),
                UserCredentialUse::File,
                UserCredentialInjectionConfig::File,
            ),
            VaultUseTarget::CommandEnvironment { origin, .. }
            | VaultUseTarget::CommandStdin { origin } => (
                host_of(origin)?,
                UserCredentialUse::Env,
                UserCredentialInjectionConfig::Env,
            ),
            _ => return Err(error("this copy destination is not supported")),
        };
        let credential = UserCredentialConfig {
            id: handle.clone(),
            description: args.label.clone().or_else(|| {
                Some(format!(
                    "Copy of {} for {}",
                    source.description.as_deref().unwrap_or(&source.id),
                    destination_word(&args.use_binding.target)
                ))
            }),
            source: UserCredentialSourceConfig::Vault {
                key: handle.clone(),
            },
            allowed_hosts: vec![allowed_host],
            allowed_uses: vec![allowed_use],
            injection,
            metadata: Some(UserCredentialMetadataConfig {
                payment_card: None,
                use_binding: Some(args.use_binding.clone()),
                created_by_kind: Some(COPIED_KIND.into()),
                created_by_id: Some(actor.agent_id.clone()),
                session_id: Some(actor.session_id.clone()),
                provider: None,
                provider_run_id: Some(actor.provider_run_id.clone()),
                vault_key: Some(handle.clone()),
                created_at_ms: Some(now),
                updated_at_ms: Some(now),
                copied_from: Some(args.from.clone()),
            }),
        };
        let _mutation = self.vault_observation_mutation_guard().await;
        recheck()?;
        self.upsert_vault_credential_locked_for_agent(
            &self.home_runtime_secret_service()?,
            &registry,
            credential,
            &value,
            false,
            Some(recheck),
        )
        .await?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"credential_id": handle, "from": args.from, "created": true}),
        })
    }
}

/// An entry for one HTTPS API: it sends `Authorization: Bearer <value>` to exactly that origin.
pub(in crate::runtime::state) fn site_bearer_use(
    origin: &str,
) -> Result<(String, UserCredentialUse, UserCredentialInjectionConfig), DaemonError> {
    Ok((
        host_of(origin)?,
        UserCredentialUse::Http,
        UserCredentialInjectionConfig::Header {
            name: "Authorization".into(),
            value: "Bearer ${secret}".into(),
        },
    ))
}

fn destination_word(target: &VaultUseTarget) -> String {
    match target {
        VaultUseTarget::Site { origin }
        | VaultUseTarget::CommandStdin { origin }
        | VaultUseTarget::CommandEnvironment { origin, .. } => origin.clone(),
        VaultUseTarget::File { path } => path.clone(),
        _ => String::new(),
    }
}

fn host_of(origin: &str) -> Result<String, DaemonError> {
    let url = url::Url::parse(origin).map_err(|_| error("the origin is not a URL"))?;
    service_host(&match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_owned(),
        _ => return Err(error("the origin has no host")),
    })
}

fn validate_copy_binding(
    run: &crate::provider::RuntimeProviderRun,
    binding: &crate::config::VaultUseBinding,
) -> Result<(), DaemonError> {
    match &binding.target {
        VaultUseTarget::Site { .. } => Ok(()),
        _ => validate_capture_binding(run, binding),
    }
}

/// A retry returns the committed handle only to the same agent, session, destination and source.
fn committed(
    existing: &UserCredentialConfig,
    turn: &VaultActor,
    from: &str,
    binding: &crate::config::VaultUseBinding,
) -> Result<RuntimeToolResult, DaemonError> {
    let metadata = existing.metadata.as_ref();
    if metadata.and_then(|m| m.created_by_kind.as_deref()) != Some(COPIED_KIND)
        || metadata.and_then(|m| m.created_by_id.as_deref()) != Some(turn.agent_id.as_str())
        || metadata.and_then(|m| m.session_id.as_deref()) != Some(turn.session_id.as_str())
        || metadata.and_then(|m| m.use_binding.as_ref()) != Some(binding)
    {
        return Err(error(
            "request_id already names a different credential; use a new request_id",
        ));
    }
    if metadata.and_then(|m| m.copied_from.as_deref()) != Some(from) {
        return Err(error(
            "request_id already made a copy of a different entry; use a new request_id",
        ));
    }
    Ok(RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({"credential_id": existing.id, "from": from, "created": false}),
    })
}

/// Deterministic per request so a lost acknowledgement never stores a second value.
fn copied_handle(turn: &VaultActor, request_id: &str) -> String {
    let digest = Sha256::digest(
        [
            COPIED_KIND,
            turn.owner_user_id.as_str(),
            &turn.session_id,
            &turn.agent_id,
            request_id.trim(),
        ]
        .join("\n"),
    );
    format!("cpy-{}", &format!("{digest:x}")[..24])
}
