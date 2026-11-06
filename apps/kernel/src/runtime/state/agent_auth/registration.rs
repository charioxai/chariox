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
    /// Every other reply field; only fields the agent names in `return_fields` ever leave.
    #[serde(flatten)]
    rest: serde_json::Map<String, serde_json::Value>,
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
    pub fields: ReturnedFields,
}

/// Non-secret registration fields the agent asked for, keyed by their JSON pointer.
#[derive(Default)]
pub(super) struct ReturnedFields {
    pub values: std::collections::BTreeMap<String, serde_json::Value>,
    pub unavailable: Vec<String>,
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
        fields: Default::default(),
    })
}

/// Words that make a pointer segment a secret. The agent picks the fields, but a service must not be able to
/// hand the model a credential through this tool.
const SECRET_SEGMENTS: [&str; 7] = [
    "token",
    "secret",
    "assertion",
    "credential",
    "password",
    "key",
    "jwt",
];

/// Checked before any network call, so a refused request never leaves an account behind.
pub(super) fn refuse_secret_fields(pointers: &[String]) -> Result<(), DaemonError> {
    let named_secret = pointers.iter().any(|pointer| {
        pointer.split('/').any(|segment| {
            let segment = segment.to_ascii_lowercase();
            SECRET_SEGMENTS.iter().any(|word| segment.contains(word))
        })
    });
    if named_secret {
        return Err(error(
            "return_fields cannot name a token, key, secret or credential field",
        ));
    }
    Ok(())
}

/// Short scalar fields of the registration reply named by JSON pointer. Services need some of them to use
/// the account, for example the id of the project the registration created. A field that is absent or not
/// a short scalar is reported, not fatal: the account already exists and its credential must still be stored.
fn requested_fields(
    reply: &serde_json::Map<String, serde_json::Value>,
    pointers: &[String],
) -> ReturnedFields {
    let reply = serde_json::Value::Object(reply.clone());
    let mut returned = ReturnedFields::default();
    for pointer in pointers {
        match reply.pointer(pointer) {
            Some(found @ (serde_json::Value::Number(_) | serde_json::Value::Bool(_))) => {
                returned.values.insert(pointer.clone(), found.clone());
            }
            Some(found @ serde_json::Value::String(text))
                if text.len() <= 256 && !text.chars().any(char::is_control) =>
            {
                returned.values.insert(pointer.clone(), found.clone());
            }
            _ => returned.unavailable.push(pointer.clone()),
        }
    }
    returned
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
    let fields = requested_fields(&registration.rest, &args.return_fields);
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
        .await
        .map(|issued| IssuedCredential { fields, ..issued });
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
        )
        .map(|issued| IssuedCredential { fields, ..issued });
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
    .map(|issued| IssuedCredential { fields, ..issued })
}
