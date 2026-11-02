//! MP-08 / MP-09 / MP-10 / MP-11: ordinary, exact-origin Vault generation. Only metadata and password
//! policy enter the tool; the generated value goes from the kernel CSPRNG
//! straight into the Vault and the model receives an opaque handle.
use super::vault_use::VaultAgentContext;
use super::*;
use crate::config::{UserCredentialConfig, UserCredentialMetadataConfig};
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
    origin: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default = "default_length")]
    length: usize,
    #[serde(default = "default_symbols")]
    symbols: bool,
}

fn default_length() -> usize {
    24
}

fn default_symbols() -> bool {
    true
}

pub(crate) fn vault_generate_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: VAULT_GENERATE.into(),
        description: "Generate a random password for one exact site origin without sudo or task approval and store it only in the owner's Vault. Returns an opaque credential_id, never the password. Fill it with chariox.kernel_browser_paste_secret into an observed password field, then click the observed submit button. Retrying with the same request_id returns the same credential.".into(),
        input_schema: serde_json::json!({"type":"object","required":["request_id","origin"],"properties":{
            "request_id":{"type":"string","minLength":1,"maxLength":128},
            "origin":{"type":"string","description":"Site origin such as https://example.com"},
            "description":{"type":"string","maxLength":200},
            "length":{"type":"integer","minimum":16,"maximum":128,"default":24},
            "symbols":{"type":"boolean","default":true}},"additionalProperties":false}),
    }
}

impl KernelRuntimeState {
    pub(crate) async fn vault_generate(
        &self,
        token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let context = self.vault_agent_context(token)?;
        let args: GenerateArgs = serde_json::from_value(arguments)
            .map_err(|_| error("invalid Vault generation arguments"))?;
        if args.request_id.trim().is_empty() || args.request_id.len() > 128 {
            return Err(error("request_id must be 1-128 characters"));
        }
        if !(16..=128).contains(&args.length) {
            return Err(error("generated password length must be 16-128"));
        }
        let origin = generation_origin(&args.origin)?;
        if args
            .description
            .as_ref()
            .is_some_and(|description| description.len() > 200)
        {
            return Err(error("description must be at most 200 characters"));
        }
        let handle = generated_handle(
            &context.owner_user_id,
            &context.session_id,
            &context.agent_id,
            &args.request_id,
        );
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        if let Some(existing) = registry.get(&handle)? {
            return committed(&existing, &context, &origin);
        }
        let _unlock = self
            .ensure_vault_unlocked_for_agent(
                &context.session_id,
                &context.agent_id,
                "vault_generate",
            )
            .await?;
        self.recheck_vault_agent(&context)?;
        let now = crate::session::unix_epoch_ms();
        let credential = UserCredentialConfig {
            id: handle.clone(),
            description: args.description,
            source: crate::config::UserCredentialSourceConfig::Vault {
                key: handle.clone(),
            },
            allowed_hosts: vec![origin.clone()],
            allowed_uses: vec![crate::config::UserCredentialUse::Browser],
            injection: crate::config::UserCredentialInjectionConfig::Browser,
            metadata: Some(UserCredentialMetadataConfig {
                use_binding: Some(crate::config::VaultUseBinding {
                    target: crate::config::VaultUseTarget::site(&origin)?,
                    value_kind: crate::config::VaultValueKind::Secret,
                }),
                created_by_kind: Some(GENERATED_KIND.into()),
                created_by_id: Some(context.agent_id.clone()),
                session_id: Some(context.session_id.clone()),
                provider: None,
                provider_run_id: Some(context.run.id().into()),
                vault_key: Some(handle.clone()),
                created_at_ms: Some(now),
                updated_at_ms: Some(now),
            }),
        };
        let secret = zeroize::Zeroizing::new(generate_password(args.length, args.symbols));
        let _mutation = self.vault_observation_mutation_guard().await;
        let stored = self
            .upsert_vault_credential_locked_for_agent(
                &self.home_runtime_secret_service()?,
                &registry,
                credential,
                secret.as_str(),
                false,
                Some(&context),
            )
            .await;
        match stored {
            Ok(_) => Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({"credential_id": handle, "origin": origin, "created": true}),
            }),
            // A concurrent retry committed first: return that handle, never a new value.
            Err(stored) => match registry.get(&handle)? {
                Some(existing) => committed(&existing, &context, &origin),
                None => Err(stored),
            },
        }
    }
}

/// Retries preserve creation identity and binding; uses are bound only to the destination.
fn committed(
    existing: &UserCredentialConfig,
    context: &VaultAgentContext,
    origin: &str,
) -> Result<RuntimeToolResult, DaemonError> {
    let metadata = existing.metadata.as_ref();
    if metadata.and_then(|m| m.created_by_kind.as_deref()) != Some(GENERATED_KIND)
        || metadata.and_then(|m| m.created_by_id.as_deref()) != Some(context.agent_id.as_str())
        || metadata.and_then(|m| m.session_id.as_deref()) != Some(context.session_id.as_str())
        || existing.allowed_hosts != [origin]
        || existing
            .metadata
            .as_ref()
            .and_then(|m| m.use_binding.as_ref())
            .is_none_or(|binding| {
                binding
                    .require(&crate::config::VaultUseTarget::Site {
                        origin: origin.into(),
                    })
                    .is_err()
            })
    {
        return Err(error(
            "request_id already names a different credential; use a new request_id",
        ));
    }
    Ok(RuntimeToolResult {
        ok: true,
        payload: serde_json::json!({"credential_id": existing.id, "origin": origin, "created": false}),
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
