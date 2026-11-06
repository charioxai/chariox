//! VAUDIT S3: move secrets between kernel-browser pages and the Vault without
//! the model reading them. `capture_browser` reveals, reads and protects a
//! value a page shows (a revealed API key, a webhook signing secret, what a
//! copy button copies) in one kernel operation under the exclusive observation
//! barrier, so no model-bound frame, snapshot or mirror of the revealed state
//! is produced before its protection. `fill_field` puts a Vault value into a
//! plain field (an env var value box) that stays masked for the model.
use super::kernel_browser_runtime::host_error;
use super::kernel_browser_secret_runtime::{require_vault_owner, SecretTarget};
use super::vault_use::VaultAgentContext;
use super::*;
use crate::config::{UserCredentialConfig, UserCredentialMetadataConfig};
use crate::runtime::kernel_browser_host::{KernelBrowserAdmission, KernelBrowserHost};
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(super) const CAPTURE: &str = "chariox.vault.capture_browser";
pub(super) const FILL: &str = "chariox.vault.fill_field";
// Shared with VAUDIT S2 capture_command: one kind for every captured value.
const CAPTURED_KIND: &str = "vault_capture";
const VALUE_LIMIT: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureArgs {
    request_id: String,
    tab_id: String,
    generation: u64,
    document_id: String,
    node_ref: String,
    #[serde(default)]
    reveal_node_ref: Option<String>,
    #[serde(default)]
    read: CaptureRead,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    use_on: Vec<String>,
    /// The value is an authenticator seed (base32 key or otpauth URI), not a password or key.
    #[serde(default)]
    totp: bool,
    /// One command environment, stdin or workspace file destination instead of a site.
    #[serde(default)]
    use_binding: Option<crate::config::VaultUseBinding>,
}

#[derive(Deserialize, Default, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
enum CaptureRead {
    /// The value is the node's text, or an input's value.
    #[default]
    Node,
    /// node_ref is a copy button; the value is what it copies.
    Clipboard,
    /// node_ref is a QR code image; the value is the text it encodes.
    Qr,
}

impl CaptureRead {
    fn wire(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Clipboard => "clipboard",
            Self::Qr => "qr",
        }
    }
}

pub(super) fn capture_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: CAPTURE.into(),
        description: "store a secret a user-domain page shows (an API key, a webhook signing secret) in the owner's Vault without reading it. Point node_ref at the element that shows the value (text or input); if the page hides it behind a Reveal/Show button, pass that button as reveal_node_ref and do not click it yourself. For a copy-to-clipboard button, pass the button as node_ref with read=clipboard. To enrol an authenticator, set totp=true and point node_ref at the manual setup key text (or at the QR code image with read=qr): the value is stored as a TOTP seed and the kernel computes codes when you fill the observed code field with the credential. The kernel reveals, reads and masks the value for you; you receive only credential_id and label. use_on declares at most one exact site origin where the credential may be filled later (default: this page's origin). Instead of a site, use_binding declares one exact command environment variable, stdin or workspace file for chariox.vault.use_env / chariox.vault.write_file. Retrying with the same request_id returns the same credential.".into(),
        input_schema: json!({"type":"object","required":["request_id","tab_id","generation","document_id","node_ref"],"properties":{
            "request_id":{"type":"string","minLength":1,"maxLength":128},
            "tab_id":{"type":"string"},"generation":{"type":"integer","minimum":1},"document_id":{"type":"string"},
            "node_ref":{"type":"string"},"reveal_node_ref":{"type":"string"},
            "read":{"type":"string","enum":["node","clipboard","qr"],"default":"node"},
            "totp":{"type":"boolean","default":false},
            "use_binding": super::vault_command::capture_binding_schema(),
            "label":{"type":"string","maxLength":200},
            "use_on":{"type":"array","maxItems":1,"items":{"type":"string","description":"Site origin such as https://vercel.com"}}},"additionalProperties":false}),
    }
}

pub(super) fn fill_spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: FILL.into(),
        description: "put a Vault credential into an observed plain text field of a user-domain tab (input, textarea or contenteditable, e.g. an environment variable value box) on a site the credential allows. The field stays masked in everything you observe. Use tab/generation/document_id/node_ref from a fresh snapshot. For login password fields use chariox.kernel_browser_paste_secret.".into(),
        input_schema: json!({"type":"object","properties":{"credential_id":{"type":"string"},"tab_id":{"type":"string"},"generation":{"type":"integer","minimum":1},"document_id":{"type":"string"},"node_ref":{"type":"string"},"submit":{"type":"boolean","default":false}},"required":["credential_id","tab_id","generation","document_id","node_ref"],"additionalProperties":false}),
    }
}

impl KernelRuntimeState {
    pub(super) async fn kernel_browser_fill_field(
        &self,
        token: &str,
        session: &str,
        agent: &crate::agent::AgentInstance,
        arguments: Value,
        admission: KernelBrowserAdmission,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let context = self.vault_agent_context(token)?;
        self.recheck_vault_agent(&context)?;
        self.kernel_browser_paste_secret(session, agent, arguments, admission, true)
            .await
    }

    pub(super) async fn kernel_browser_capture(
        &self,
        token: &str,
        session: &str,
        agent: &crate::agent::AgentInstance,
        arguments: Value,
        admission: KernelBrowserAdmission,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let turn = self.vault_agent_context(token)?;
        let args: CaptureArgs = serde_json::from_value(arguments)
            .map_err(|_| host_error("VAUDIT: invalid Vault capture arguments".into()))?;
        if args.request_id.trim().is_empty() || args.request_id.len() > 128 {
            return Err(host_error("request_id must be 1-128 characters".into()));
        }
        if args.use_binding.is_some() && (args.totp || !args.use_on.is_empty()) {
            return Err(host_error(
                "use_binding replaces use_on and totp; pass only one declared use".into(),
            ));
        }
        if args.label.as_ref().is_some_and(|label| label.len() > 200) || args.use_on.len() > 1 {
            return Err(host_error(
                "label is at most 200 characters and use_on at most one origin".into(),
            ));
        }
        let user = self.provider_account_authority_owner_user_id(agent.owner_user_id());
        let user = user.as_str();
        require_vault_owner(user)?;
        let check_authority = || {
            self.owned
                .kernel_browser_host
                .check_admission(Some(&admission))
                .map_err(host_error)
        };
        check_authority()?;
        let target = SecretTarget {
            tab_id: &args.tab_id,
            generation: args.generation,
            document_id: &args.document_id,
            node_ref: &args.node_ref,
            reveal_node_ref: args.reveal_node_ref.as_deref(),
        };
        let url = self
            .kernel_browser_secret_target_url(user, &admission, &target, false)
            .await?;
        let source = crate::config::vault_use::vault_origin(&url)?;
        let use_on = vec![match args.use_on.first() {
            Some(origin) => crate::config::vault_use::vault_origin(origin)?,
            None => source.clone(),
        }];
        let use_target = if args.totp {
            crate::config::VaultUseTarget::Totp {
                origin: use_on[0].clone(),
            }
        } else {
            crate::config::VaultUseTarget::Site {
                origin: use_on[0].clone(),
            }
        };
        let handle = captured_handle(&turn, &args.request_id);
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        let destination = match &args.use_binding {
            Some(binding) => {
                // Checked before anything is revealed, so a refusal costs no secret.
                super::vault_command::validate_capture_binding(&turn.run, binding)?;
                let host = super::vault_command::service_host(
                    source
                        .trim_start_matches("https://")
                        .trim_start_matches("http://"),
                )?;
                Some((binding.clone(), host))
            }
            None => None,
        };
        if let Some(existing) = registry.get(&handle)? {
            return match &destination {
                Some((binding, host)) => {
                    super::vault_command::committed_capture(&existing, &turn, host, binding)
                }
                None => committed(&existing, &turn, &use_on, &use_target),
            };
        }
        let protection = &self.owned.kernel_browser_secret_observations;
        let scope = KernelBrowserHost::profile_key(user);
        let epoch = protection.revision(&scope)?;
        // Unlock before revealing: a key a service shows once must not be lost.
        let _unlock = self
            .ensure_vault_unlocked_for_agent(session, agent.id(), "vault_capture_browser")
            .await?;
        self.recheck_vault_agent(&turn)?;
        check_authority()?;
        let secret = {
            let _barrier = protection.barrier(&scope)?.write_owned().await;
            check_authority()?;
            if protection.revision(&scope)? != epoch {
                return Err(host_error(
                    "MP-11: observation protection changed during the Vault wait; observe the page again".into(),
                ));
            }
            if self
                .kernel_browser_secret_target_url_bound(user, &admission, &target, false)
                .await?
                != url
            {
                return Err(host_error(
                    "VAUDIT: the page changed during the Vault wait; observe it again".into(),
                ));
            }
            let snapshot = self
                .kernel_browser_bound_operation(
                    user,
                    Some(&admission),
                    "host.browser",
                    json!({"op":"snapshot","tab_id":args.tab_id,"generation":args.generation}),
                    true,
                )
                .await?;
            let target_id = snapshot["snapshot"]["target_id"]
                .as_str()
                .ok_or_else(|| host_error("MD-5: missing target identity".into()))?
                .to_string();
            let mut params = json!({"op":"capture","tab_id":args.tab_id,"generation":args.generation,
                "document_id":args.document_id,"node_ref":args.node_ref,"reveal_node_ref":args.reveal_node_ref,
                "read":args.read.wire(),"expected_document_url":url});
            self.claim_app_view_tab(user, &admission, &mut params)?;
            check_authority()?;
            let mut read = self
                .kernel_browser_bound_operation(user, Some(&admission), "host.secret", params, true)
                .await?;
            let secret = match read["value"].take() {
                Value::String(value) if args.read == CaptureRead::Qr => {
                    let image = base64::engine::general_purpose::STANDARD
                        .decode(value)
                        .map_err(|_| host_error("VAUDIT: the page returned no QR image".into()))?;
                    super::kernel_browser_qr::decode_png_qr(&image)?
                }
                Value::String(value) => zeroize::Zeroizing::new(value),
                _ => return Err(host_error("VAUDIT: the page returned no value".into())),
            };
            // Protect whatever was read before the barrier opens, even a value
            // refused below: the reveal may have put it on the page.
            if !secret.is_empty() && secret.len() <= VALUE_LIMIT {
                protection.register_credential_source(&scope, Some(&handle))?;
                protection.register_capture(
                    &scope,
                    &target_id,
                    &args.document_id,
                    &args.node_ref,
                    &secret,
                )?;
            }
            captured_value(&secret)?;
            secret
        };
        // The element may hold a sentence around the key ("Or enter the secret: ABCD …").
        let secret = if args.totp {
            crate::secret::locate_totp_key(&secret).ok_or_else(|| {
                host_error(
                    "the element holds no authenticator setup key; point node_ref at the manual key text or the QR image (read=qr). Nothing was stored".into(),
                )
            })?
        } else {
            secret
        };
        let now = crate::session::unix_epoch_ms();
        let label = args.label.clone().unwrap_or_else(|| {
            format!(
                "Captured from {}",
                url::Url::parse(&source)
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .unwrap_or_else(|| source.clone())
            )
        });
        if let Some((use_binding, host)) = destination {
            return self
                .store_captured(
                    &turn,
                    &registry,
                    super::vault_command::Captured {
                        handle,
                        host,
                        label: Some(label),
                        use_binding,
                    },
                    &secret,
                )
                .await;
        }
        let credential = UserCredentialConfig {
            id: handle.clone(),
            description: Some(label.clone()),
            source: crate::config::UserCredentialSourceConfig::Vault {
                key: handle.clone(),
            },
            allowed_hosts: use_on.clone(),
            allowed_uses: vec![crate::config::UserCredentialUse::Browser],
            injection: if args.totp {
                crate::config::UserCredentialInjectionConfig::Totp(
                    crate::config::TotpConfig::default(),
                )
            } else {
                crate::config::UserCredentialInjectionConfig::Browser
            },
            metadata: Some(UserCredentialMetadataConfig {
                use_binding: Some(crate::config::VaultUseBinding {
                    target: use_target.clone(),
                    value_kind: crate::config::VaultValueKind::Secret,
                }),
                created_by_kind: Some(CAPTURED_KIND.into()),
                created_by_id: Some(turn.agent_id.clone()),
                session_id: Some(turn.session_id.clone()),
                provider: None,
                provider_run_id: Some(turn.run.id().to_string()),
                vault_key: Some(handle.clone()),
                created_at_ms: Some(now),
                updated_at_ms: Some(now),
            }),
        };
        let _mutation = self.vault_observation_mutation_guard().await;
        match self
            .upsert_vault_credential_locked_for_agent(
                &self.home_runtime_secret_service()?,
                &registry,
                credential,
                secret.as_str(),
                false,
                Some(&turn),
            )
            .await
        {
            Ok(_) => Ok(RuntimeToolResult {
                ok: true,
                payload: json!({"credential_id": handle, "label": label, "use_on": use_on,
                    "totp": args.totp, "length": secret.chars().count(), "created": true}),
            }),
            // A concurrent retry committed first: return that handle, never a new value.
            Err(stored) => match registry.get(&handle)? {
                Some(existing) => committed(&existing, &turn, &use_on, &use_target),
                None => Err(stored),
            },
        }
    }
}

/// A value still shown masked (bullets, asterisks) is not the secret.
fn captured_value(value: &str) -> Result<(), DaemonError> {
    if value.is_empty() || value.len() > VALUE_LIMIT {
        return Err(host_error(
            "VAUDIT: the captured value is empty or longer than 16 KiB; nothing was stored".into(),
        ));
    }
    if value
        .chars()
        .any(|c| matches!(c, '\u{2022}' | '\u{25CF}' | '\u{2219}' | '\u{00B7}'))
        || value.chars().all(|c| c == '*')
    {
        return Err(host_error(
            "VAUDIT: the element still shows a masked value; pass its Reveal/Show button as reveal_node_ref. Nothing was stored".into(),
        ));
    }
    Ok(())
}

/// A retry returns the committed handle only to the same agent, session and sites.
fn committed(
    existing: &UserCredentialConfig,
    turn: &VaultAgentContext,
    use_on: &[String],
    target: &crate::config::VaultUseTarget,
) -> Result<RuntimeToolResult, DaemonError> {
    let metadata = existing.metadata.as_ref();
    if metadata.and_then(|m| m.created_by_kind.as_deref()) != Some(CAPTURED_KIND)
        || metadata.and_then(|m| m.created_by_id.as_deref()) != Some(turn.agent_id.as_str())
        || metadata.and_then(|m| m.session_id.as_deref()) != Some(turn.session_id.as_str())
        || existing.allowed_hosts != use_on
        || metadata
            .and_then(|m| m.use_binding.as_ref())
            .is_none_or(|binding| {
                binding.target != *target
                    || binding.value_kind != crate::config::VaultValueKind::Secret
            })
    {
        return Err(host_error(
            "request_id already names a different credential; use a new request_id".into(),
        ));
    }
    Ok(RuntimeToolResult {
        ok: true,
        payload: json!({"credential_id": existing.id, "label": existing.description,
            "use_on": use_on, "totp": matches!(target, crate::config::VaultUseTarget::Totp { .. }), "created": false}),
    })
}

/// Deterministic per request so a lost acknowledgement never stores a second value.
fn captured_handle(turn: &VaultAgentContext, request_id: &str) -> String {
    let digest = Sha256::digest(
        [
            CAPTURED_KIND,
            "browser",
            turn.owner_user_id.as_str(),
            &turn.session_id,
            &turn.agent_id,
            request_id.trim(),
        ]
        .join("\n"),
    );
    format!("cap-{}", &format!("{digest:x}")[..24])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_never_takes_secret_bytes_and_refuses_masked_values() {
        let arguments = json!({"request_id":"r","tab_id":"t","generation":1,"document_id":"d","node_ref":"backend:1","value":"sk_test_x"});
        assert!(serde_json::from_value::<CaptureArgs>(arguments).is_err());
        for spec in [capture_spec(), fill_spec()] {
            for forbidden in ["value", "text", "secret"] {
                assert!(spec.input_schema["properties"].get(forbidden).is_none());
            }
        }
        let read: CaptureArgs = serde_json::from_value(json!({"request_id":"r","tab_id":"t","generation":1,"document_id":"d","node_ref":"backend:1","read":"clipboard"})).unwrap();
        assert_eq!(read.read, CaptureRead::Clipboard);
        assert!(captured_value("sk_test_\u{2022}\u{2022}\u{2022}\u{2022}").is_err());
        assert!(captured_value("********").is_err());
        assert!(captured_value("").is_err());
        assert!(captured_value(&"a".repeat(VALUE_LIMIT + 1)).is_err());
        assert!(captured_value("whsec_live_example_value").is_ok());
    }
}
