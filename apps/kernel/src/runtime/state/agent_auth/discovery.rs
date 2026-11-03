//! RFC 9728 resource metadata -> RFC 8414 AS metadata -> auth.md.
//! Markdown is fetched as documentation, never evaluated as a program.
use super::*;

#[derive(serde::Deserialize)]
struct ResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
    #[serde(default)]
    bearer_methods_supported: Vec<String>,
}

#[derive(serde::Deserialize)]
struct ServerMetadata {
    issuer: String,
    token_endpoint: Option<String>,
    agent_auth: AgentAuthMetadata,
}

#[derive(serde::Deserialize)]
struct AgentAuthMetadata {
    skill: Option<String>,
    identity_endpoint: Option<String>,
    register_uri: Option<String>,
    claim_endpoint: Option<String>,
    #[serde(default)]
    identity_types_supported: Vec<String>,
}

pub(super) struct Discovery {
    pub resource: String,
    pub identity_endpoint: String,
    pub token_endpoint: Option<String>,
    pub claim_endpoint: Option<String>,
}

fn well_known(base: &url::Url, name: &str) -> String {
    let path = base.path().trim_end_matches('/');
    format!(
        "{}/.well-known/{name}{path}",
        base.origin().ascii_serialization()
    )
}

impl Discovery {
    pub async fn fetch(
        http: &AuthHttp,
        args: &RegisterArgs,
        flow: Flow,
    ) -> Result<Self, DaemonError> {
        let requested = &http.authority().resource;
        let metadata_url = args
            .resource_metadata_url
            .clone()
            .unwrap_or_else(|| well_known(requested, "oauth-protected-resource"));
        let reply = http.get(&metadata_url).await?;
        reply.success()?;
        let prm: ResourceMetadata = reply.json()?;
        let resource = http::endpoint(&prm.resource)?;
        if resource.origin() != requested.origin()
            || resource.path().trim_end_matches('/') != requested.path().trim_end_matches('/')
            || resource.query() != requested.query()
        {
            return Err(error("auth.md resource metadata does not match resource_url; use the service's canonical resource URL"));
        }
        if !prm.bearer_methods_supported.is_empty()
            && !prm
                .bearer_methods_supported
                .iter()
                .any(|method| method == "header")
        {
            return Err(error(
                "auth.md service does not support Bearer header credentials",
            ));
        }
        if !prm.scopes_supported.is_empty()
            && args
                .scopes
                .iter()
                .any(|scope| !prm.scopes_supported.contains(scope))
        {
            return Err(error(
                "requested auth.md scopes are not advertised by this resource",
            ));
        }
        let server = prm
            .authorization_servers
            .first()
            .ok_or_else(|| error("auth.md resource metadata has no authorization server"))?;
        let issuer = http::endpoint(server)?;
        let reply = http
            .get(&well_known(&issuer, "oauth-authorization-server"))
            .await?;
        reply.success()?;
        let metadata: ServerMetadata = reply.json()?;
        if metadata.issuer.trim_end_matches('/') != issuer.as_str().trim_end_matches('/') {
            return Err(error(
                "auth.md authorization-server issuer does not match resource metadata",
            ));
        }
        let method = if flow == Flow::Anonymous {
            "anonymous"
        } else {
            "service_auth"
        };
        // Missing identity_types_supported is informational per the spec. If present,
        // an explicit exclusion is actionable without trying a registration side effect.
        if !metadata.agent_auth.identity_types_supported.is_empty()
            && !metadata
                .agent_auth
                .identity_types_supported
                .iter()
                .any(|value| value == method)
        {
            return Err(error("auth.md service does not advertise the requested issuer-free flow; choose a service that supports Anonymous or Service Auth"));
        }
        let metadata_origin = http::endpoint(&metadata_url)?
            .origin()
            .ascii_serialization();
        let skill = metadata
            .agent_auth
            .skill
            .unwrap_or_else(|| format!("{metadata_origin}/auth.md"));
        let skill_reply = http.get(&skill).await?;
        skill_reply.success()?;
        if skill_reply.body.trim().is_empty() {
            return Err(error("the service's auth.md is empty"));
        }
        let identity_endpoint = match (
            metadata.agent_auth.identity_endpoint,
            metadata.agent_auth.register_uri,
        ) {
            (Some(endpoint), None) | (None, Some(endpoint)) => endpoint,
            _ => {
                return Err(error(
                    "auth.md metadata must name one registration endpoint",
                ))
            }
        };
        let result = Self {
            resource: resource.to_string(),
            identity_endpoint,
            token_endpoint: metadata.token_endpoint,
            claim_endpoint: metadata.agent_auth.claim_endpoint,
        };
        for endpoint in std::iter::once(&result.identity_endpoint)
            .chain(result.token_endpoint.iter())
            .chain(result.claim_endpoint.iter())
        {
            let url = http::endpoint(endpoint)?;
            if requested.scheme() == "https" && url.scheme() != "https" {
                return Err(error(
                    "auth.md metadata cannot downgrade an HTTPS resource to HTTP",
                ));
            }
        }
        Ok(result)
    }
}
