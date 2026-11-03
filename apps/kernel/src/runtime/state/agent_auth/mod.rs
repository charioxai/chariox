//! MP-08 / MP-09 / MP-10 / MP-11 AU-15: issuer-free auth.md registration.
//! Service replies stay kernel-private; only a Vault handle and caller label leave.
use super::*;
use crate::config::{UserCredentialConfig, UserCredentialMetadataConfig};
use crate::runtime::kernel_access::error;
use crate::transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec};
use serde_json::json;

mod claim;
mod discovery;
mod http;
mod registration;
mod storage;
use discovery::Discovery;
use http::AuthHttp;
use registration::IssuedCredential;
pub(super) use storage::RegistrationClaims;

pub(super) const REGISTER: &str = "chariox.agent_auth.register";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterArgs {
    request_id: String,
    resource_url: String,
    resource_metadata_url: Option<String>,
    label: Option<String>,
    email: Option<String>,
    flow: Option<Flow>,
    #[serde(default)]
    claim: bool,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    return_fields: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Flow {
    Anonymous,
    ServiceAuth,
}

pub(super) fn spec() -> RuntimeToolSpec {
    RuntimeToolSpec {
        name: REGISTER.into(),
        description: "Register at an auth.md service using Anonymous or Service Auth, without a trusted identity issuer. Discovers OAuth resource/server metadata and the service's auth.md. Service Auth (email) or claim:true opens one owner claim popup while the kernel polls. Credentials go directly into the owner's Vault; returns only credential_id and label. Use the handle with http_request_with_credential. Reuse request_id after a lost reply. Identity assertions from an agent provider are not supported.".into(),
        input_schema: json!({"type":"object","required":["request_id","resource_url"],"properties":{
            "request_id":{"type":"string","minLength":1,"maxLength":128},
            "resource_url":{"type":"string","description":"Canonical resource URL from the service's OAuth protected-resource metadata"},
            "resource_metadata_url":{"type":"string","description":"Optional resource_metadata URL advertised in WWW-Authenticate; otherwise discover at the conventional well-known path"},
            "label":{"type":"string","maxLength":200},
            "email":{"type":"string","maxLength":320,"description":"User login_hint; selects Service Auth unless flow is explicit"},
            "flow":{"enum":["anonymous","service_auth"]},
            "claim":{"type":"boolean","default":false,"description":"Also claim an anonymous registration; requires email"},
            "scopes":{"type":"array","maxItems":64,"items":{"type":"string","minLength":1,"maxLength":200}},
            "return_fields":{"type":"array","maxItems":8,"items":{"type":"string","minLength":2,"maxLength":128},"description":"JSON pointers of short non-secret fields of the service's registration reply to return, for example /project/id when the service scopes its API to the project the registration created. Fields naming a token, key, secret or credential are refused."}
        },"additionalProperties":false}),
    }
}

impl RegisterArgs {
    fn validate(&self) -> Result<(url::Url, Flow), DaemonError> {
        if self.request_id.trim().is_empty()
            || self.request_id.len() > 128
            || self
                .label
                .as_ref()
                .is_some_and(|label| label.len() > 200 || label.chars().any(char::is_control))
            || self.email.as_ref().is_some_and(|email| {
                email.trim().is_empty() || email.len() > 320 || email.chars().any(char::is_control)
            })
            || self.return_fields.len() > 8
            || self
                .return_fields
                .iter()
                .any(|pointer| !pointer.starts_with('/') || pointer.len() > 128)
            || self.scopes.len() > 64
            || self.scopes.iter().any(|scope| {
                scope.is_empty()
                    || scope.len() > 200
                    || scope.chars().any(char::is_whitespace)
                    || scope.chars().any(char::is_control)
            })
        {
            return Err(error("invalid auth.md registration arguments"));
        }
        registration::refuse_secret_fields(&self.return_fields)?;
        let resource = http::endpoint(&self.resource_url)?;
        let flow = self.flow.unwrap_or(if self.email.is_some() {
            Flow::ServiceAuth
        } else {
            Flow::Anonymous
        });
        if (flow == Flow::ServiceAuth || self.claim) && self.email.is_none() {
            return Err(error(
                "Service Auth and anonymous claim require the user's email",
            ));
        }
        Ok((resource, flow))
    }
}

/// Build the network authority from the shared Vault admission.
/// All network steps and the post-wait Vault commit pass through the same adapter.
fn registration_authority(
    state: &KernelRuntimeState,
    token: &str,
    _resource: &url::Url,
) -> Result<RegistrationAuthority, DaemonError> {
    let turn = state.vault_agent_context(token)?;
    Ok(RegistrationAuthority {
        context: turn.clone(),
        actor: RegistrationActor {
            owner_user_id: turn.owner_user_id,
            session_id: turn.session_id,
            agent_id: turn.agent_id,
            prompt_id: turn.prompt_id,
            provider_run_id: Some(turn.run.id().into()),
        },
    })
}

#[derive(Clone, PartialEq)]
struct RegistrationActor {
    owner_user_id: String,
    session_id: String,
    agent_id: String,
    prompt_id: Option<String>,
    provider_run_id: Option<String>,
}

struct RegistrationAuthority {
    context: super::vault_use::VaultAgentContext,
    actor: RegistrationActor,
}

#[derive(Clone)]
struct Authority {
    state: KernelRuntimeState,
    token: String,
    resource: url::Url,
    actor: RegistrationActor,
    context: super::vault_use::VaultAgentContext,
}

impl Authority {
    fn check(&self) -> Result<(), DaemonError> {
        let current = registration_authority(&self.state, &self.token, &self.resource)?;
        if current.actor != self.actor {
            return Err(error(
                "auth.md authorization changed; restart registration in the current task",
            ));
        }
        Ok(())
    }
}

impl KernelRuntimeState {
    pub(super) async fn agent_auth_register(
        &self,
        token: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let args: RegisterArgs = serde_json::from_value(arguments)
            .map_err(|_| error("invalid auth.md registration arguments"))?;
        let (resource, flow) = args.validate()?;
        let registration = registration_authority(self, token, &resource)?;
        let authority = Authority {
            state: self.clone(),
            token: token.into(),
            actor: registration.actor,
            context: registration.context,
            resource,
        };
        let handle = storage::handle(&authority.actor, &args.request_id);
        let _claim = self.owned.agent_auth_registrations.claim(&handle)?;
        let registry = crate::credential::CharioxCredentialRegistry::user()?;
        let label = args.label.clone().unwrap_or_else(|| {
            format!(
                "auth.md for {}",
                authority.resource.origin().ascii_serialization()
            )
        });
        if let Some(existing) = registry.get(&handle)? {
            return storage::committed(&existing, &authority, &label);
        }
        // Unlock before a remote side effect so a declined Vault prompt cannot orphan an account.
        let _unlock = self
            .ensure_vault_unlocked_for_agent(
                &authority.actor.session_id,
                &authority.actor.agent_id,
                "agent_auth_register",
            )
            .await?;
        authority.check()?;
        let http = AuthHttp::new(authority.clone());
        let discovery = Discovery::fetch(&http, &args, flow).await?;
        let issued = registration::register(&http, &discovery, &args, flow).await?;
        authority.check()?;
        self.store_agent_auth_credential(&authority, &registry, &handle, &label, issued)
            .await
    }
}
