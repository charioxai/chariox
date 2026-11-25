//! MP-08 / MP-09 / MP-10 / MP-11: ordinary, exact-origin Vault generation. Only metadata and password
//! policy enter the tool; the generated value goes from the kernel CSPRNG
//! straight into the Vault and the model receives an opaque handle.
use super::vault_use::{VaultActor, VaultRecheck};
use super::*;
use crate::config::{
    UserCredentialConfig, UserCredentialInjectionConfig, UserCredentialMetadataConfig,
    UserCredentialUse, VaultUseBinding, VaultUseTarget,
};
use crate::runtime::kernel_access::error;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
use rand::Rng;
use sha2::{Digest, Sha256};

pub(crate) const VAULT_GENERATE: &str = "chariox.vault.generate";
const GENERATED_KIND: &str = "vault_generate";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerateArgs {
    request_id: String,
    /// A site password for this one origin.
    #[serde(default)]
    origin: Option<String>,
    /// A random value for one command environment, command stdin or workspace file.
    #[serde(default)]
    use_binding: Option<VaultUseBinding>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    length: Option<usize>,
    #[serde(default)]
    symbols: Option<bool>,
    #[serde(default)]
    format: GenerateFormat,
}

#[derive(serde::Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum GenerateFormat {
    #[default]
    Password,
    Hex,
}

/// What the entry is for: a site's password field, or one command/file use.
enum Destination {
    Site(String),
    Command(VaultUseBinding),
}

impl GenerateArgs {
    fn destination(&self) -> Result<Destination, DaemonError> {
        match (&self.origin, &self.use_binding) {
            (Some(origin), None) => Ok(Destination::Site(generation_origin(origin)?)),
            (None, Some(binding)) => {
                if binding.value_kind != crate::config::VaultValueKind::Secret
                    || !matches!(
                        binding.target,
                        VaultUseTarget::CommandEnvironment { .. }
                            | VaultUseTarget::CommandStdin { .. }
                            | VaultUseTarget::File { .. }
                    )
                {
                    return Err(error(
                        "use_binding must name a command environment, command stdin or workspace file; for a site password pass origin instead",
                    ));
                }
                binding.validate()?;
                Ok(Destination::Command(binding.clone()))
            }
            _ => Err(error(
                "give exactly one of origin (a password for one site) or use_binding (a random value for a command environment, stdin or file)",
            )),
        }
    }

    /// Command values default to no symbols: `$` and quotes break shells and config files.
    fn policy(&self, destination: &Destination) -> (usize, bool) {
        let command = matches!(destination, Destination::Command(_));
        (
            self.length.unwrap_or(if self.format == GenerateFormat::Hex { 64 } else { 24 }),
            self.symbols.unwrap_or(!command),
        )
    }
}

pub(crate) fn vault_generate_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: VAULT_GENERATE.into(),
        description: "Generate a random value and store it only in the owner's Vault, without sudo or approval. Returns an opaque credential_id, never the value. Give exactly one of: origin (a password for one exact site; fill it with chariox.kernel_browser_paste_secret), or use_binding (a random secret for one command environment variable, stdin or workspace file, used at exactly that destination with chariox.vault.use_env or write_file). format hex makes length hex characters (default 64). Command values have no symbols unless symbols is true. The same request_id returns the same credential.".into(),
        input_schema: serde_json::json!({"type":"object","required":["request_id"],"properties":{
            "request_id":{"type":"string","minLength":1,"maxLength":128},
            "origin":{"type":"string","description":"Site origin such as https://example.com"},
            "use_binding":{"type":"object","description":"As in capture_command: {target:{kind:command_environment,name,origin} | {kind:command_stdin,origin} | {kind:file,path}}"},
            "format":{"type":"string","enum":["password","hex"],"default":"password"},
            "description":{"type":"string","maxLength":200},
            "length":{"type":"integer","minimum":16,"maximum":128,"description":"default 24, hex 64"},
            "symbols":{"type":"boolean","description":"default true for a site, false for a command"}},"additionalProperties":false}),
    }
}

impl KernelRuntimeState {
    pub(crate) async fn vault_generate(
        &self,
        token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let context = self.vault_leased_context(token).await?;
        // A file destination is this workspace's: check it here, where the home cannot.
        let args: GenerateArgs = serde_json::from_value(arguments.clone())
            .map_err(|e| error(format!("invalid Vault generation arguments: {e}")))?;
        if let Destination::Command(binding) = args.destination()? {
            super::vault_command::validate_capture_binding(&context.run, &binding)?;
        }
        if let Some(home) = context.home.clone() {
            return self
                .forward_vault_tool_to_home(home, VAULT_GENERATE, arguments)
                .await;
        }
        self.generate_in_vault(
            &context.actor(),
            &|| self.recheck_vault_agent(&context),
            arguments,
        )
        .await
    }

    /// Generates for `actor` in the Vault of this kernel: the owner's own, or the home's for an
    /// agent on a leased worker.
    pub(super) async fn generate_in_vault(
        &self,
        actor: &VaultActor,
        recheck: VaultRecheck<'_>,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let args: GenerateArgs = serde_json::from_value(arguments)
            .map_err(|e| error(format!("invalid Vault generation arguments: {e}")))?;
        if args.request_id.trim().is_empty() || args.request_id.len() > 128 {
            return Err(error("request_id must be 1-128 characters"));
        }
        let destination = args.destination()?;
        let (length, symbols) = args.policy(&destination);
        if !(16..=128).contains(&length) {
            return Err(error("generated value length must be 16-128"));
        }
        if args
            .description
            .as_ref()
            .is_some_and(|description| description.len() > 200)
        {
            return Err(error("description must be at most 200 characters"));
        }
        let handle = generated_handle(
            &actor.owner_user_id,
            &actor.session_id,
            &actor.agent_id,
            &args.request_id,
        );
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        if let Some(existing) = registry.get(&handle)? {
            self.record_vault_resource(&existing)?;
            return committed(&existing, actor, &destination);
        }
        let _unlock = self
            .ensure_vault_unlocked_for_agent(&actor.session_id, &actor.agent_id, "vault_generate")
            .await?;
        recheck()?;
        let now = crate::session::unix_epoch_ms();
        let (binding, allowed_host, allowed_use, injection) = destination.entry()?;
        let credential = UserCredentialConfig {
            id: handle.clone(),
            description: args.description,
            source: crate::config::UserCredentialSourceConfig::Vault {
                key: handle.clone(),
            },
            allowed_hosts: vec![allowed_host],
            allowed_uses: vec![allowed_use],
            injection,
            metadata: Some(UserCredentialMetadataConfig {
                payment_card: None,
                use_binding: Some(binding),
                created_by_kind: Some(GENERATED_KIND.into()),
                created_by_id: Some(actor.agent_id.clone()),
                session_id: Some(actor.session_id.clone()),
                provider: None,
                provider_run_id: Some(actor.provider_run_id.clone()),
                vault_key: Some(handle.clone()),
                created_at_ms: Some(now),
                updated_at_ms: Some(now),
                copied_from: None,
            }),
        };
        let secret = zeroize::Zeroizing::new(match args.format {
            GenerateFormat::Password => generate_password(length, symbols),
            GenerateFormat::Hex => generate_hex(length),
        });
        let _mutation = self.vault_observation_mutation_guard().await;
        let stored = self
            .upsert_vault_credential_locked_for_agent(
                &self.home_runtime_secret_service()?,
                &registry,
                credential,
                secret.as_str(),
                false,
                Some(recheck),
            )
            .await;
        match stored {
            Ok(_) => Ok(RuntimeToolResult {
                ok: true,
                payload: result_payload(&handle, &destination, true),
            }),
            // A concurrent retry committed first: return that handle, never a new value.
            Err(stored) => match registry.get(&handle)? {
                Some(existing) => {
                    self.record_vault_resource(&existing)?;
                    committed(&existing, actor, &destination)
                }
                None => Err(stored),
            },
        }
    }
}

impl Destination {
    /// The binding and how the Vault entry is allowed to be used for it.
    fn entry(
        &self,
    ) -> Result<
        (
            VaultUseBinding,
            String,
            UserCredentialUse,
            UserCredentialInjectionConfig,
        ),
        DaemonError,
    > {
        Ok(match self {
            Destination::Site(origin) => (
                VaultUseBinding {
                    target: VaultUseTarget::site(origin)?,
                    value_kind: crate::config::VaultValueKind::Secret,
                },
                origin.clone(),
                UserCredentialUse::Browser,
                UserCredentialInjectionConfig::Browser,
            ),
            Destination::Command(binding) => match &binding.target {
                VaultUseTarget::File { .. } => (
                    binding.clone(),
                    "localhost".to_owned(),
                    UserCredentialUse::File,
                    UserCredentialInjectionConfig::File,
                ),
                VaultUseTarget::CommandEnvironment { origin, .. }
                | VaultUseTarget::CommandStdin { origin } => (
                    binding.clone(),
                    super::vault_command::service_host(&origin_host(origin)?)?,
                    UserCredentialUse::Env,
                    UserCredentialInjectionConfig::Env,
                ),
                _ => return Err(error("this generate destination is not supported")),
            },
        })
    }

    fn binding(&self) -> Result<VaultUseBinding, DaemonError> {
        self.entry().map(|(binding, ..)| binding)
    }
}

fn origin_host(origin: &str) -> Result<String, DaemonError> {
    let url = url::Url::parse(origin).map_err(|_| error("the origin is not a URL"))?;
    Ok(match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_owned(),
        _ => return Err(error("the origin has no host")),
    })
}

fn result_payload(handle: &str, destination: &Destination, created: bool) -> serde_json::Value {
    match destination {
        Destination::Site(origin) => {
            serde_json::json!({"credential_id": handle, "origin": origin, "created": created})
        }
        Destination::Command(binding) => {
            serde_json::json!({"credential_id": handle, "use_binding": binding, "created": created})
        }
    }
}

/// Retries preserve creation identity and binding; uses are bound only to the destination.
fn committed(
    existing: &UserCredentialConfig,
    actor: &VaultActor,
    destination: &Destination,
) -> Result<RuntimeToolResult, DaemonError> {
    let metadata = existing.metadata.as_ref();
    let wanted = destination.binding()?;
    if metadata.and_then(|m| m.created_by_kind.as_deref()) != Some(GENERATED_KIND)
        || metadata.and_then(|m| m.created_by_id.as_deref()) != Some(actor.agent_id.as_str())
        || metadata.and_then(|m| m.session_id.as_deref()) != Some(actor.session_id.as_str())
        || metadata.and_then(|m| m.use_binding.as_ref()) != Some(&wanted)
    {
        return Err(error(
            "request_id already names a different credential; use a new request_id",
        ));
    }
    Ok(RuntimeToolResult {
        ok: true,
        payload: result_payload(&existing.id, destination, false),
    })
}

/// Deterministic per request so a lost acknowledgement never mints a second value.
pub(super) fn generated_handle(
    owner: &str,
    session: &str,
    agent: &str,
    request_id: &str,
) -> String {
    let digest = Sha256::digest([owner, session, agent, request_id.trim()].join("\n"));
    format!("gen-{}", &format!("{digest:x}")[..24])
}

fn generation_origin(raw: &str) -> Result<String, DaemonError> {
    crate::config::vault_use::vault_origin(raw)
}

/// `length` lowercase hex characters from the kernel CSPRNG.
fn generate_hex(length: usize) -> String {
    let mut rng = rand::rngs::OsRng;
    (0..length)
        .map(|_| char::from_digit(rng.gen_range(0..16), 16).expect("digit below 16"))
        .collect()
}

pub(in crate::runtime::state) fn generate_password(length: usize, symbols: bool) -> String {
    const CLASSES: [&str; 4] = [
        "abcdefghijkmnopqrstuvwxyz",
        "ABCDEFGHJKLMNPQRSTUVWXYZ",
        "23456789",
        "!#$%&*+-=?@^_",
    ];
    let classes = if symbols { &CLASSES[..] } else { &CLASSES[..3] };
    let alphabet = classes.concat().chars().collect::<Vec<_>>();
    let mut rng = rand::rngs::OsRng;
    // Sites commonly require every class; resample until each one is present.
    loop {
        let password = (0..length)
            .map(|_| alphabet[rng.gen_range(0..alphabet.len())])
            .collect::<String>();
        if classes
            .iter()
            .all(|class| password.chars().any(|c| class.contains(c)))
        {
            return password;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_binds_secure_origins_and_meets_every_class() {
        assert_eq!(
            generation_origin("https://example.com/signup").unwrap(),
            "https://example.com"
        );
        assert_eq!(
            generation_origin("http://127.0.0.1:8123").unwrap(),
            "http://127.0.0.1:8123"
        );
        for refused in [
            "http://example.com",
            "ftp://example.com",
            "https://user:pw@example.com",
            "not a url",
        ] {
            assert!(generation_origin(refused).is_err(), "{refused}");
        }
        for _ in 0..64 {
            let password = generate_password(16, true);
            assert_eq!(password.chars().count(), 16);
            assert!(password.chars().any(|c| c.is_ascii_digit()));
            assert!(password.chars().any(|c| !c.is_ascii_alphanumeric()));
            assert!(generate_password(16, false)
                .chars()
                .all(|c| c.is_ascii_alphanumeric()));
        }
    }
}
