//! MP-08/MP-10/MP-11 A06: elevated Vault generation and the handle/origin
//! fences shared with kernel-side protected login. Only metadata and password
//! policy enter the tool; the generated value goes from the kernel CSPRNG
//! straight into the Vault and the model receives an opaque handle.
use super::*;
use crate::config::{UserCredentialConfig, UserCredentialMetadataConfig};
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
        description: "During your sudo window, generate a random password for one site and store it only in the owner's Vault. Returns an opaque credential_id, never the password. Fill it with chariox.kernel_browser_paste_secret into an observed password field, then click the observed submit button. Retrying with the same request_id returns the same credential.".into(),
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
        let turn = self
            .sudo_for_auth_token(token)
            .map_err(|_| error("Vault generation requires a live sudo window"))?;
        let args: GenerateArgs = serde_json::from_value(arguments)
            .map_err(|_| error("invalid Vault generation arguments"))?;
        if args.request_id.trim().is_empty() || args.request_id.len() > 128 {
            return Err(error("request_id must be 1-128 characters"));
        }
        if !(16..=128).contains(&args.length) {
            return Err(error("generated password length must be 16-128"));
        }
        let origin = generation_origin(&args.origin)?;
        // An enrolled kernel's owner acts as its Cloud user; map it as MD-5 does.
        if self.provider_account_authority_owner_user_id(&turn.owner_user_id)
            != crate::session::DEFAULT_LOCAL_USER_ID
            || self
                .owned
                .config_projection
                .snapshot()
                .user_config
                .credential_vault
                .agent_management
                == crate::config::CredentialVaultAgentManagementPolicy::Deny
        {
            return Err(error("the owner's Vault is not available to this agent"));
        }
        let handle = generated_handle(&turn, &args.request_id);
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        if let Some(existing) = registry.get(&handle)? {
            return committed(&existing, &turn, &origin);
        }
        let _unlock = self
            .ensure_vault_unlocked_for_agent(&turn.session_id, &turn.agent_id, "vault_generate")
            .await?;
        // The unlock wait can outlast, revoke or replace the window.
        if self
            .sudo_for_auth_token(token)
            .ok()
            .is_none_or(|current| current.entry_id != turn.entry_id)
        {
            return Err(error("sudo window ended during the Vault wait"));
        }
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
                created_by_kind: Some(GENERATED_KIND.into()),
                created_by_id: Some(turn.agent_id.clone()),
                session_id: Some(turn.session_id.clone()),
                provider: None,
                provider_run_id: turn.provider_run_id.clone(),
                vault_key: Some(handle.clone()),
                created_at_ms: Some(now),
                updated_at_ms: Some(now),
            }),
        };
        let secret = zeroize::Zeroizing::new(generate_password(args.length, args.symbols));
        // MP-08/MP-10/MP-11 review #937 P1: retain the submitting turn across
        // lifecycle/barrier/app waits; the shared pre-write check sees this binding.
        let authority = LocalDaemonRequest::ListSessions(crate::local::ListSessionsRequest);
        let bound = self
            .with_external_command_authority(Some((&turn.entry_id, &authority)))
            .with_sudo_binding(turn.prompt_id.clone().zip(turn.provider_run_id.clone()));
        let stored = bound
            .upsert_observed_vault_credential(
                &self.home_runtime_secret_service()?,
                &registry,
                credential,
                secret.as_str(),
                false,
            )
            .await;
        match stored {
            Ok(_) => Ok(RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({"credential_id": handle, "origin": origin, "created": true}),
            }),
            // A concurrent retry committed first: return that handle, never a new value.
            Err(stored) => match registry.get(&handle)? {
                Some(existing) => committed(&existing, &turn, &origin),
                None => Err(stored),
            },
        }
    }
}

/// A retry returns the committed handle only to the same window's agent and site.
fn committed(
    existing: &UserCredentialConfig,
    turn: &KernelSudoTurn,
    origin: &str,
) -> Result<RuntimeToolResult, DaemonError> {
    let metadata = existing.metadata.as_ref();
    if metadata.and_then(|m| m.created_by_kind.as_deref()) != Some(GENERATED_KIND)
        || metadata.and_then(|m| m.created_by_id.as_deref()) != Some(turn.agent_id.as_str())
        || metadata.and_then(|m| m.session_id.as_deref()) != Some(turn.session_id.as_str())
        || existing.allowed_hosts != [origin]
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
fn generated_handle(turn: &KernelSudoTurn, request_id: &str) -> String {
    let digest = Sha256::digest(
        [
            turn.owner_user_id.as_str(),
            &turn.session_id,
            &turn.agent_id,
            request_id.trim(),
        ]
        .join("\n"),
    );
    format!("gen-{}", &format!("{digest:x}")[..24])
}

/// HTTPS origins, or HTTP on loopback only; login binds the canonical origin.
fn generation_origin(raw: &str) -> Result<String, DaemonError> {
    let url = url::Url::parse(raw.trim()).map_err(|_| error("origin must be a URL"))?;
    let host = url.host_str().unwrap_or_default().to_string();
    let loopback = host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if host.is_empty()
        || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(error("origin must be https (or http on loopback)"));
    }
    Ok(url.origin().ascii_serialization())
}

fn generate_password(length: usize, symbols: bool) -> String {
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

/// Login fences on the handle itself. Generated handles stay inside their
/// session; an elevated (unfocused) fill also needs an explicit site binding.
pub(crate) fn require_login_handle_scope(
    credential: &UserCredentialConfig,
    session: &str,
    elevated: bool,
) -> Result<(), DaemonError> {
    let metadata = credential.metadata.as_ref();
    if metadata.and_then(|m| m.created_by_kind.as_deref()) == Some(GENERATED_KIND)
        && metadata.and_then(|m| m.session_id.as_deref()) != Some(session)
    {
        return Err(error("this credential belongs to another session"));
    }
    if elevated && credential.allowed_hosts.is_empty() {
        return Err(error(
            "elevated Vault fill requires a credential bound to its site",
        ));
    }
    Ok(())
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
