//! `chariox.vault.capture_command`: a command prints a secret (an API key a
//! CLI just created, `openssl rand` output); the kernel stores it in the Vault
//! and the agent receives only the handle and its own label.
use super::*;
use crate::config::{UserCredentialConfig, UserCredentialMetadataConfig};
use crate::runtime::state::runtime_vault_unlock_state::VaultUnlockGuard;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub(crate) const VAULT_CAPTURE_COMMAND: &str = "chariox.vault.capture_command";
pub(super) const CAPTURED_KIND: &str = "vault_capture";
const VALUE_LIMIT: usize = 16 * 1024;

#[derive(Clone, Default)]
pub(in crate::runtime::state) struct CaptureClaims {
    claims: Arc<std::sync::Mutex<BTreeMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
    /// The unlock of a capture whose command runs on a leased worker, kept until the worker
    /// sends the value, so "ask every time" asks once and a value a service shows once is not lost.
    /// The id names the hold its expiry belongs to.
    unlocks: Arc<std::sync::Mutex<BTreeMap<String, (u64, VaultUnlockGuard)>>>,
}

/// How long a worker's capture command may run, with room for the transfer back.
const CAPTURE_UNLOCK_HOLD: Duration = Duration::from_secs(MAX_TIMEOUT_SECONDS + 60);

impl CaptureClaims {
    pub(super) async fn claim(
        &self,
        handle: &str,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, DaemonError> {
        let claim = {
            let mut claims = self
                .claims
                .lock()
                .map_err(|_| error("Vault capture claims unavailable"))?;
            claims.retain(|_, claim| claim.strong_count() > 0);
            if let Some(claim) = claims.get(handle).and_then(std::sync::Weak::upgrade) {
                claim
            } else {
                let claim = Arc::new(tokio::sync::Mutex::new(()));
                claims.insert(handle.to_owned(), Arc::downgrade(&claim));
                claim
            }
        };
        Ok(claim.lock_owned().await)
    }

    /// Callers hold the handle's claim and took any earlier hold, so nothing is replaced.
    fn hold_unlock(&self, handle: &str, unlock: VaultUnlockGuard) {
        let id = rand::random::<u64>();
        if let Ok(mut unlocks) = self.unlocks.lock() {
            unlocks.insert(handle.to_owned(), (id, unlock));
        }
        let (unlocks, handle) = (self.unlocks.clone(), handle.to_owned());
        tokio::spawn(async move {
            tokio::time::sleep(CAPTURE_UNLOCK_HOLD).await;
            // A retry holds the unlock anew; only this hold's own expiry ends it.
            let lapsed = unlocks.lock().ok().and_then(|mut unlocks| {
                let held = unlocks.get(&handle).is_some_and(|(held, _)| *held == id);
                held.then(|| unlocks.remove(&handle)).flatten()
            });
            drop(lapsed);
        });
    }

    fn take_unlock(&self, handle: &str) -> Option<VaultUnlockGuard> {
        let (_, unlock) = self.unlocks.lock().ok()?.remove(handle)?;
        Some(unlock)
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureArgs {
    request_id: String,
    argv: serde_json::Value,
    #[serde(default)]
    cwd: Option<String>,
    host: String,
    /// The service origin the command uses the `env` entries for, as in `chariox.vault.use_env`.
    #[serde(default)]
    origin: Option<String>,
    /// Existing Vault entries the command runs with: variable name -> credential_id.
    #[serde(default)]
    env: BTreeMap<String, String>,
    use_binding: crate::config::VaultUseBinding,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    extract: Extract,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

#[derive(serde::Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Extract {
    /// The whole output or file (see `stored_text`).
    #[default]
    #[serde(alias = "whole")]
    Stdout,
    Json {
        pointer: String,
    },
    Regex {
        pattern: String,
        #[serde(default = "first_group")]
        group: usize,
    },
}

fn first_group() -> usize {
    1
}

pub(crate) fn capture_command_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: VAULT_CAPTURE_COMMAND.into(),
        description: "run one command (argv, no shell) in your workspace and store a secret it prints on stdout in the owner's Vault. You never see the value or the command's output: the result is an opaque credential_id with your label. extract picks the value: {\"kind\":\"stdout\"} (whole stdout, the default), {\"kind\":\"json\",\"pointer\":\"/data/api_key\"} (JSON Pointer into stdout), or {\"kind\":\"regex\",\"pattern\":\"token: (\\\\S+)\",\"group\":1}. host names the source service. use_binding declares the one destination: a command environment name and origin, command stdin origin, or absolute workspace file path. Use it only at that destination with chariox.vault.use_env or chariox.vault.write_file. To run it with existing Vault entries, pass origin and env (variable name -> credential_id) as in chariox.vault.use_env; the output goes straight into the new entry. Retrying with the same request_id returns the same credential without running the command again.".into(),
        input_schema: serde_json::json!({"type":"object","required":["request_id","argv","host","use_binding"],"properties":{
            "use_binding": capture_binding_schema(),
            "request_id":{"type":"string","minLength":1,"maxLength":128},
            "argv":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":64},
            "cwd":{"type":"string","description":"Directory inside your workspace, relative to it"},
            "host":{"type":"string","description":"Host of the service the secret is for, such as api.example.com"},
            "label":{"type":"string","maxLength":200},
            "origin":{"type":"string"},
            "env":{"type":"object","additionalProperties":{"type":"string"},"maxProperties":super::use_env::MAX_VARIABLES},
            "extract":{"type":"object","required":["kind"],"properties":{
                "kind":{"type":"string","enum":["stdout","json","regex"]},
                "pointer":{"type":"string"},
                "pattern":{"type":"string","maxLength":1024},
                "group":{"type":"integer","minimum":0}},"additionalProperties":false},
            "timeout_seconds":{"type":"integer","minimum":1,"maximum":MAX_TIMEOUT_SECONDS,"default":DEFAULT_TIMEOUT_SECONDS}},
            "additionalProperties":false}),
    }
}

pub(in crate::runtime::state) fn capture_binding_schema() -> serde_json::Value {
    serde_json::json!({"type":"object","required":["target"],"additionalProperties":false,"properties":{
        "value_kind":{"type":"string","enum":["secret","payment_card"],"default":"secret"},
        "target":{"oneOf":[
            {"type":"object","required":["kind","name","origin"],"additionalProperties":false,"properties":{
                "kind":{"const":"command_environment"},"name":{"type":"string"},"origin":{"type":"string"}}},
            {"type":"object","required":["kind","origin"],"additionalProperties":false,"properties":{
                "kind":{"const":"command_stdin"},"origin":{"type":"string"}}},
            {"type":"object","required":["kind","path"],"additionalProperties":false,"properties":{
                "kind":{"const":"file"},"path":{"type":"string","description":"One absolute canonical file path inside this workspace"}}}
        ]}
    }})
}

fn env_target(
    name: &str,
    origin: Option<&str>,
) -> Result<crate::config::VaultUseTarget, DaemonError> {
    let origin =
        origin.ok_or_else(|| error("name the origin the command uses the env entries for"))?;
    Ok(crate::config::VaultUseTarget::CommandEnvironment {
        name: name.into(),
        origin: crate::config::vault_use::vault_origin(origin)?,
    })
}

/// The existing Vault entries the command runs with, checked before anything runs.
fn bound_env_uses(
    args: &CaptureArgs,
) -> Result<Vec<(String, crate::config::VaultUseTarget)>, DaemonError> {
    if args.env.is_empty() {
        return match args.origin {
            None => Ok(Vec::new()),
            Some(_) => Err(error(
                "origin is for the env entries; name them in env or drop it",
            )),
        };
    }
    if args.env.len() > super::use_env::MAX_VARIABLES {
        return Err(error(format!(
            "at most {} environment variables",
            super::use_env::MAX_VARIABLES
        )));
    }
    if let Some(name) = args
        .env
        .keys()
        .find(|name| !super::use_env::variable_name(name))
    {
        return Err(error(format!(
            "`{name}` is not an environment variable name"
        )));
    }
    args.env
        .iter()
        .map(|(name, id)| Ok((id.clone(), env_target(name, args.origin.as_deref())?)))
        .collect()
}

/// What the capture tools refuse alike on the worker and at the home.
pub(super) fn check_capture_request(
    request_id: &str,
    label: Option<&str>,
) -> Result<(), DaemonError> {
    if request_id.trim().is_empty() || request_id.len() > 128 {
        return Err(error("request_id must be 1-128 characters"));
    }
    if label.is_some_and(|label| label.len() > 200) {
        return Err(error("label must be at most 200 characters"));
    }
    Ok(())
}

impl KernelRuntimeState {
    pub(crate) async fn vault_capture_command(
        &self,
        token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let context = self.vault_leased_context(token).await?;
        self.recheck_vault_command_context(&context)?;
        let args: CaptureArgs = serde_json::from_value(arguments)
            .map_err(|e| error(format!("invalid Vault capture arguments: {e}")))?;
        check_capture_request(&args.request_id, args.label.as_deref())?;
        // The kernel owns settlement and commit even if the MCP waiter goes away.
        let state = self.clone();
        tokio::spawn(async move { state.capture_command_once(context, args).await })
            .await
            .map_err(|_| error("the capture runner stopped unexpectedly"))?
    }

    async fn capture_command_once(
        &self,
        context: VaultAgentContext,
        args: CaptureArgs,
    ) -> Result<RuntimeToolResult, DaemonError> {
        args.use_binding.target.validate()?;
        let host = service_host(&args.host)?;
        let argv = super::super::agent_process_watch::command_argv(&args.argv, false)?;
        let pattern = extract_pattern(&args.extract)?;
        let timeout = command_timeout(args.timeout_seconds)?;
        let env_uses = bound_env_uses(&args)?;
        let actor = context.actor();
        let handle = captured_handle(&actor, &args.request_id);
        let _claim = self.owned.vault_capture_claims.claim(&handle).await?;
        self.recheck_vault_command_context(&context)?;
        let recheck = || self.recheck_vault_command_context(&context);
        let _unlock = match &context.home {
            // The home keeps the Vault: it answers a retry and unlocks before the command runs.
            Some(home) => {
                if let Some(committed) = self
                    .prepare_home_vault_capture(
                        home,
                        &args.request_id,
                        &host,
                        &args.use_binding,
                        &env_uses,
                    )
                    .await?
                {
                    return Ok(committed);
                }
                validate_capture_binding(&context.run, &args.use_binding)?;
                None
            }
            None => {
                let registry = crate::credential::CharioxCredentialRegistry::user()?;
                if let Some(existing) = registry.get(&handle)? {
                    return committed(&existing, &actor, &host, &args.use_binding);
                }
                validate_capture_binding(&context.run, &args.use_binding)?;
                // Refused before anything is unlocked or run: an unknown entry or one not made for this origin.
                for (id, target) in &env_uses {
                    self.preflight_workspace_vault_use(&context.agent_id, id, target)?;
                }
                Some(self.capture_unlock(&actor).await?)
            }
        };
        let cwd =
            super::super::agent_process_watch::workspace_cwd(&context.run, args.cwd.as_deref())?;
        recheck()?;
        let mut env = Vec::new();
        let mut masks = Vec::new();
        for (name, id) in &args.env {
            let target = env_target(name, args.origin.as_deref())?;
            let value = self
                .read_workspace_vault_value(&context, id, &target)
                .await?;
            masks.push((id.clone(), value.clone()));
            env.push((name.clone(), value));
        }
        recheck()?;
        // Longest first, so a value containing another is masked whole.
        masks.sort_by_key(|(_, value)| std::cmp::Reverse(value.len()));
        let output = run_command(
            self,
            context.clone(),
            super::super::agent_process_watch::KernelCommand {
                argv,
                cwd,
                env,
                stdin: None,
                timeout,
            },
        )
        .await?;
        use super::super::agent_process_watch::KernelCommandExit;
        match output.exit {
            KernelCommandExit::Exited(0) => {}
            KernelCommandExit::Exited(code) => {
                return Err(error(format!(
                    "the command exited with code {code}; nothing was stored{}",
                    stderr_excerpt(super::use_env::masked(&output.stderr, &masks).0.as_bytes())
                )))
            }
            KernelCommandExit::TimedOut => {
                return Err(error(format!(
                    "the command did not finish within {} s and was stopped; nothing was stored",
                    timeout.as_secs()
                )))
            }
            KernelCommandExit::Lost => {
                return Err(error(
                    "the command could not be settled; nothing was stored",
                ))
            }
        }
        if output.truncated {
            return Err(error(
                "the command printed more than 1 MiB; nothing was stored",
            ));
        }
        let secret = extract(&output.stdout, &args.extract, pattern.as_ref())?;
        self.store_capture(
            &context,
            Captured {
                handle,
                host,
                label: args.label,
                use_binding: args.use_binding,
                request_id: args.request_id,
            },
            &secret,
        )
        .await
    }

    /// Unlock before running: a key a service shows once must not be lost.
    pub(super) async fn capture_unlock(
        &self,
        actor: &VaultActor,
    ) -> Result<VaultUnlockGuard, DaemonError> {
        let unlock = self
            .ensure_vault_unlocked_for_agent(&actor.session_id, &actor.agent_id, "vault_capture")
            .await?;
        unlock.keep_open_for(CAPTURE_UNLOCK_HOLD)?;
        Ok(unlock)
    }

    /// Stores a captured value where the Vault is: here, or at the home for an agent on a
    /// leased worker.
    pub(in crate::runtime::state) async fn store_capture(
        &self,
        context: &VaultAgentContext,
        captured: Captured,
        secret: &str,
    ) -> Result<RuntimeToolResult, DaemonError> {
        match &context.home {
            Some(home) => self.store_home_vault_capture(home, captured, secret).await,
            None => {
                let registry = crate::credential::CharioxCredentialRegistry::user()?;
                self.store_captured(
                    &context.actor(),
                    &|| self.recheck_vault_command_context(context),
                    &registry,
                    captured,
                    secret,
                )
                .await
            }
        }
    }

    /// The home half of a capture on a leased worker, before the worker runs its command:
    /// the committed handle of a retry, or else the Vault unlocked and kept so for the store.
    pub(super) async fn prepare_capture_in_vault(
        &self,
        actor: &VaultActor,
        recheck: VaultRecheck<'_>,
        request_id: &str,
        host: &str,
        use_binding: &crate::config::VaultUseBinding,
    ) -> Result<Option<RuntimeToolResult>, DaemonError> {
        let handle = captured_handle(actor, request_id);
        let _claim = self.owned.vault_capture_claims.claim(&handle).await?;
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        if let Some(existing) = registry.get(&handle)? {
            return committed(&existing, actor, host, use_binding).map(Some);
        }
        // A retry after a failed command keeps the unlock the first attempt held: a fresh
        // guard on the still open Vault does not lock, and replacing the held one would lock now.
        let unlock = match self
            .owned
            .vault_capture_claims
            .take_unlock(&handle)
            .filter(VaultUnlockGuard::still_unlocked)
        {
            Some(held) => {
                held.keep_open_for(CAPTURE_UNLOCK_HOLD)?;
                held
            }
            None => self.capture_unlock(actor).await?,
        };
        recheck()?;
        self.owned.vault_capture_claims.hold_unlock(&handle, unlock);
        Ok(None)
    }

    /// The home half of a capture on a leased worker, once the worker has the value.
    pub(super) async fn store_capture_in_vault(
        &self,
        actor: &VaultActor,
        recheck: VaultRecheck<'_>,
        captured: Captured,
        secret: &str,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let _claim = self
            .owned
            .vault_capture_claims
            .claim(&captured.handle)
            .await?;
        recheck()?;
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        if let Some(existing) = registry.get(&captured.handle)? {
            return committed(&existing, actor, &captured.host, &captured.use_binding);
        }
        let _unlock = match self
            .owned
            .vault_capture_claims
            .take_unlock(&captured.handle)
            .filter(VaultUnlockGuard::still_unlocked)
        {
            Some(unlock) => unlock,
            None => self.capture_unlock(actor).await?,
        };
        recheck()?;
        self.store_captured(actor, recheck, &registry, captured, secret)
            .await
    }

    /// Stores a captured value for kernel-local use (command env/stdin and files).
    pub(in crate::runtime::state) async fn store_captured(
        &self,
        actor: &VaultActor,
        recheck: VaultRecheck<'_>,
        registry: &crate::credential::CharioxCredentialRegistry,
        captured: Captured,
        secret: &str,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let turn = actor;
        let now = crate::session::unix_epoch_ms();
        let credential = UserCredentialConfig {
            id: captured.handle.clone(),
            description: captured.label.clone(),
            source: crate::config::UserCredentialSourceConfig::Vault {
                key: captured.handle.clone(),
            },
            allowed_hosts: vec![captured.host.clone()],
            allowed_uses: vec![match captured.use_binding.target {
                crate::config::VaultUseTarget::File { .. } => {
                    crate::config::UserCredentialUse::File
                }
                _ => crate::config::UserCredentialUse::Env,
            }],
            injection: match captured.use_binding.target {
                crate::config::VaultUseTarget::File { .. } => {
                    crate::config::UserCredentialInjectionConfig::File
                }
                _ => crate::config::UserCredentialInjectionConfig::Env,
            },
            metadata: Some(UserCredentialMetadataConfig {
                payment_card: None,
                use_binding: Some(captured.use_binding),
                created_by_kind: Some(CAPTURED_KIND.into()),
                created_by_id: Some(turn.agent_id.clone()),
                session_id: Some(turn.session_id.clone()),
                provider: None,
                provider_run_id: Some(turn.provider_run_id.clone()),
                vault_key: Some(captured.handle.clone()),
                created_at_ms: Some(now),
                updated_at_ms: Some(now),
                copied_from: None,
            }),
        };
        let _mutation = self.vault_observation_mutation_guard().await;
        recheck()?;
        self.upsert_vault_credential_locked_for_agent(
            &self.home_runtime_secret_service()?,
            registry,
            credential,
            secret,
            false,
            Some(recheck),
        )
        .await?;
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({"credential_id": captured.handle, "label": captured.label,
                "host": captured.host, "length": secret.chars().count(), "created": true}),
        })
    }
}

pub(in crate::runtime::state) struct Captured {
    /// The handle in the kernel the Vault is at; a leased run's home derives its own from `request_id`.
    pub(in crate::runtime::state) handle: String,
    pub(in crate::runtime::state) host: String,
    pub(in crate::runtime::state) label: Option<String>,
    pub(in crate::runtime::state) use_binding: crate::config::VaultUseBinding,
    pub(in crate::runtime::state) request_id: String,
}

pub(super) fn extract_pattern(how: &Extract) -> Result<Option<regex::Regex>, DaemonError> {
    let Extract::Regex { pattern, group } = how else {
        return Ok(None);
    };
    let regex = regex::RegexBuilder::new(pattern)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| error(format!("invalid extract pattern: {e}")))?;
    if *group >= regex.captures_len() {
        return Err(error("extract group is not in the pattern"));
    }
    Ok(Some(regex))
}

/// A retry returns the committed handle only to the same agent, session and host.
pub(in crate::runtime::state) fn committed(
    existing: &UserCredentialConfig,
    turn: &VaultActor,
    host: &str,
    binding: &crate::config::VaultUseBinding,
) -> Result<RuntimeToolResult, DaemonError> {
    let metadata = existing.metadata.as_ref();
    if metadata.and_then(|m| m.created_by_kind.as_deref()) != Some(CAPTURED_KIND)
        || metadata.and_then(|m| m.created_by_id.as_deref()) != Some(turn.agent_id.as_str())
        || metadata.and_then(|m| m.session_id.as_deref()) != Some(turn.session_id.as_str())
        || existing.allowed_hosts != [host]
        || metadata.and_then(|m| m.use_binding.as_ref()) != Some(binding)
    {
        return Err(error(
            "request_id already names a different credential; use a new request_id",
        ));
    }
    Ok(RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({"credential_id": existing.id, "label": existing.description,
            "host": host, "created": false}),
    })
}

/// Deterministic per request so a lost acknowledgement never stores a second value.
pub(super) fn captured_handle(turn: &VaultActor, request_id: &str) -> String {
    let digest = Sha256::digest(
        [
            CAPTURED_KIND,
            turn.owner_user_id.as_str(),
            &turn.session_id,
            &turn.agent_id,
            request_id.trim(),
        ]
        .join("\n"),
    );
    format!("cap-{}", &format!("{digest:x}")[..24])
}

/// A bare host name (with an optional port), as credential hosts are matched.
pub(in crate::runtime::state) fn service_host(raw: &str) -> Result<String, DaemonError> {
    let raw = raw.trim();
    let url = url::Url::parse(&format!("https://{raw}"))
        .ok()
        .filter(|url| url.path() == "/" && url.query().is_none() && url.username().is_empty())
        .filter(|_| !raw.contains(['/', '?', '#', '@']));
    match url
        .as_ref()
        .and_then(|url| url.host_str().map(|h| (h, url.port())))
    {
        Some((host, Some(port))) => Ok(format!("{host}:{port}")),
        Some((host, None)) => Ok(host.to_string()),
        None => Err(error("host must be a host name such as api.example.com")),
    }
}

/// Errors name what went wrong, never the output they looked at.
pub(super) fn extract(
    stdout: &[u8],
    how: &Extract,
    pattern: Option<&regex::Regex>,
) -> Result<Zeroizing<String>, DaemonError> {
    let text = std::str::from_utf8(stdout)
        .map_err(|_| error("the output is not UTF-8 text; nothing was stored"))?;
    let value = match how {
        Extract::Stdout => stored_text(text),
        Extract::Json { pointer } => {
            let json: serde_json::Value = serde_json::from_str(text)
                .map_err(|_| error("the output is not JSON; nothing was stored"))?;
            match json.pointer(pointer) {
                Some(serde_json::Value::String(value)) => stored_text(value),
                Some(serde_json::Value::Number(value)) => Zeroizing::new(value.to_string()),
                Some(_) => return Err(error(
                    "the JSON pointer names an object, array, boolean or null; nothing was stored",
                )),
                None => {
                    return Err(error(
                        "the JSON pointer matched nothing; nothing was stored",
                    ))
                }
            }
        }
        Extract::Regex { group, .. } => {
            let captures = pattern
                .and_then(|p| p.captures(text))
                .ok_or_else(|| error("the pattern matched nothing; nothing was stored"))?;
            stored_text(captures.get(*group).map_or("", |m| m.as_str()))
        }
    };
    if value.is_empty() {
        return Err(error("the extracted value is empty; nothing was stored"));
    }
    if value.len() > VALUE_LIMIT {
        return Err(error(
            "the extracted value is too large; nothing was stored",
        ));
    }
    Ok(value)
}

/// A one-line value (a token printed with `echo`) loses its surrounding whitespace and line
/// ending. A multi-line value (a private key, a certificate) is a text file whose final line
/// ending belongs to it: OpenSSH refuses a key without it, so it is stored exactly.
pub(super) fn stored_text(value: &str) -> Zeroizing<String> {
    let trimmed = value.trim();
    Zeroizing::new(
        if trimmed.contains('\n') {
            value
        } else {
            trimmed
        }
        .to_string(),
    )
}

/// The tail of a failed command's stderr, sanitized; stdout is never shown.
fn stderr_excerpt(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines = text
        .lines()
        .map(|line| super::super::agent_process_output::sanitize(line.as_bytes()))
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>();
    let tail = lines[lines.len().saturating_sub(5)..].join("\n");
    match tail.char_indices().rev().nth(511) {
        _ if tail.is_empty() => String::new(),
        Some((start, _)) => format!(": …{}", &tail[start..]),
        None => format!(": {tail}"),
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    fn args(origin: Option<&str>, env: &[(&str, &str)]) -> CaptureArgs {
        CaptureArgs {
            request_id: "r".into(),
            argv: serde_json::json!(["/bin/true"]),
            cwd: None,
            host: "api.example.com".into(),
            origin: origin.map(str::to_owned),
            env: env
                .iter()
                .map(|(name, id)| ((*name).to_owned(), (*id).to_owned()))
                .collect(),
            use_binding: serde_json::from_value(serde_json::json!({"target":
                {"kind":"command_environment","name":"NEW","origin":"https://api.example.com"}}))
            .unwrap(),
            label: None,
            extract: Extract::Stdout,
            timeout_seconds: None,
        }
    }

    #[test]
    fn bound_env_entries_need_an_origin_and_a_variable_name() {
        assert!(bound_env_uses(&args(None, &[])).unwrap().is_empty());
        let uses =
            bound_env_uses(&args(Some("https://api.example.com"), &[("KEY", "cap-1")])).unwrap();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].0, "cap-1");
        for refused in [
            args(None, &[("KEY", "cap-1")]),
            args(Some("https://api.example.com"), &[]),
            args(Some("https://api.example.com"), &[("BAD-NAME", "cap-1")]),
            args(Some("not an origin"), &[("KEY", "cap-1")]),
        ] {
            assert!(bound_env_uses(&refused).is_err());
        }
    }

    #[test]
    fn capture_extracts_without_echoing_and_hosts_are_bare() {
        let regex = regex::Regex::new(r"token: (\S+)").unwrap();
        let rx = Extract::Regex {
            pattern: String::new(),
            group: 1,
        };
        assert_eq!(
            extract(b"created\ntoken: sk_123\n", &rx, Some(&regex))
                .unwrap()
                .as_str(),
            "sk_123"
        );
        let json = Extract::Json {
            pointer: "/data/key".into(),
        };
        assert_eq!(
            extract(br#"{"data":{"key":" k-1 "}}"#, &json, None)
                .unwrap()
                .as_str(),
            "k-1"
        );
        assert_eq!(
            extract(b"  abc\n", &Extract::Stdout, None)
                .unwrap()
                .as_str(),
            "abc"
        );
        for (stdout, how) in [
            (&b"secret-in-output"[..], &json),
            (&b"{\"data\":{}}"[..], &json),
            (&b"   \n"[..], &Extract::Stdout),
        ] {
            let message = extract(stdout, how, None).unwrap_err().to_string();
            assert!(!message.contains("secret-in-output"), "{message}");
        }
        assert_eq!(service_host("api.example.com").unwrap(), "api.example.com");
        assert_eq!(service_host("127.0.0.1:8123").unwrap(), "127.0.0.1:8123");
        for refused in ["", "https://x.com", "x.com/path", "u@x.com", "a b"] {
            assert!(service_host(refused).is_err(), "{refused}");
        }
    }
}
