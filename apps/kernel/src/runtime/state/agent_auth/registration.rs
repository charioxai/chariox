use super::*;
use zeroize::Zeroizing;

/// No Debug/Serialize implementation: a service-created value has no output path.
pub(super) struct Secret(Zeroizing<String>);
impl<'de> serde::Deserialize<'de> for Secret {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <String as serde::Deserialize>::deserialize(deserializer)
            .map(|value| Self(Zeroizing::new(value)))
    }
}
impl Secret {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(serde::Deserialize)]
struct Registration {
    registration_id: String,
    identity_assertion: Option<Secret>,
    claim_token: Option<Secret>,
    claim: Option<super::claim::Ceremony>,
    credential: Option<Secret>,
    api_key: Option<Secret>,
    credential_type: Option<String>,
}

#[derive(serde::Deserialize)]
pub(super) struct TokenReply {
    pub access_token: Option<Secret>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
}

pub(super) struct IssuedCredential {
    pub value: Secret,
    pub registration_id: String,
    pub resource: String,
}

fn issued(
    value: Secret,
    registration_id: String,
    resource: &str,
) -> Result<IssuedCredential, DaemonError> {
    if value.as_str().is_empty()
        || value.as_str().len() > 65536
        || value.as_str().chars().any(char::is_control)
    {
        return Err(error(
            "auth.md service returned an invalid Bearer credential",
        ));
    }
    Ok(IssuedCredential {
        value,
        registration_id,
        resource: resource.into(),
    })
}

pub(super) fn token_credential(
    reply: TokenReply,
    registration_id: String,
    resource: &str,
    scopes: &[String],
) -> Result<IssuedCredential, DaemonError> {
    if reply.error.is_some()
        || !reply
            .token_type
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("bearer"))
    {
        return Err(error(
            "auth.md token exchange did not issue a Bearer credential; restart registration",
        ));
    }
    if !scopes.is_empty()
        && scopes.iter().any(|scope| {
            !reply
                .scope
                .as_deref()
                .unwrap_or_default()
                .split_whitespace()
                .any(|granted| granted == scope)
        })
    {
        return Err(error("auth.md service did not grant the requested scopes"));
    }
    issued(
        reply
            .access_token
            .ok_or_else(|| error("auth.md token response has no access_token"))?,
        registration_id,
        resource,
    )
}

pub(super) async fn register(
    http: &AuthHttp,
    discovery: &Discovery,
    args: &RegisterArgs,
    flow: Flow,
) -> Result<IssuedCredential, DaemonError> {
    let body = if flow == Flow::ServiceAuth {
        json!({"type":"service_auth", "login_hint":args.email})
    } else {
        json!({"type":"anonymous"})
    };
    let reply = http.post_json(&discovery.identity_endpoint, &body).await?;
    reply.success()?;
    let registration: Registration = reply.json()?;
    if registration.registration_id.is_empty() || registration.registration_id.len() > 256 {
        return Err(error(
            "auth.md registration response has no valid registration_id",
        ));
    }
    if flow == Flow::ServiceAuth || args.claim {
        let claim_token = registration
            .claim_token
            .ok_or_else(|| error("auth.md claim response has no claim_token"))?;
        let ceremony = match registration.claim {
            Some(ceremony) => ceremony,
            None => {
                if flow == Flow::ServiceAuth {
                    return Err(error("auth.md Service Auth response has no claim ceremony"));
                }
                let endpoint = discovery
                    .claim_endpoint
                    .as_deref()
                    .ok_or_else(|| error("auth.md metadata has no claim_endpoint"))?;
                #[derive(serde::Serialize)]
                struct ClaimRequest<'a> {
                    claim_token: &'a str,
                    email: &'a str,
                }
                let reply = http
                    .post_json(
                        endpoint,
                        &ClaimRequest {
                            claim_token: claim_token.as_str(),
                            email: args.email.as_deref().expect("validated email"),
                        },
                    )
                    .await?;
                reply.success()?;
                #[derive(serde::Deserialize)]
                struct ClaimReply {
                    claim_attempt: super::claim::Ceremony,
                }
                reply.json::<ClaimReply>()?.claim_attempt
            }
        };
        return super::claim::complete(
            http,
            discovery,
            args,
            registration.registration_id,
            claim_token,
            ceremony,
        )
        .await;
    }
    // Some auth.md services issue an API key directly, without an OAuth exchange.
    if registration.credential_type.as_deref() == Some("api_key") {
        if !args.scopes.is_empty() {
            return Err(error(
                "direct auth.md API keys do not advertise granted scopes",
            ));
        }
        return issued(
            registration
                .credential
                .or(registration.api_key)
                .ok_or_else(|| error("auth.md registration has no API key"))?,
            registration.registration_id,
            &discovery.resource,
        );
    }
    let assertion = registration
        .identity_assertion
        .ok_or_else(|| error("anonymous auth.md registration has no identity_assertion"))?;
    let endpoint = discovery
        .token_endpoint
        .as_deref()
        .ok_or_else(|| error("auth.md metadata has no token_endpoint"))?;
    let scope = args.scopes.join(" ");
    let mut pairs = vec![
        ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
        ("assertion", assertion.as_str()),
        ("resource", discovery.resource.as_str()),
    ];
    if !scope.is_empty() {
        pairs.push(("scope", &scope));
    }
    let reply = http.post_form(endpoint, &pairs).await?;
    reply.success()?;
    token_credential(
        reply.json()?,
        registration.registration_id,
        &discovery.resource,
        &args.scopes,
    )
}
