//! Immutable declared destinations plus the existing live installation fence.
//! A publisher declaration is insufficient by itself: require_current must run
//! on the existing writer before an admitted request can write to its socket.
use super::{HttpError, Result};
use chariox_app_package::{HttpMethod, VerifiedPackage};
use chariox_app_runtime::app_catalog::AppCatalog;
use hyper::{
    header::{HeaderName, HeaderValue},
    HeaderMap, Method,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    sync::Arc,
};
use url::Url;

const MAX_URL_BYTES: usize = 8192;
const MAX_HEADERS: usize = 64;
const MAX_HEADER_BYTES: usize = 16 * 1024;
pub(super) const MAX_RESOLVED_ADDRESSES: usize = 32;

pub(super) struct AppHttpPolicy {
    catalog: Arc<AppCatalog>,
    rules: Rules,
}
struct Rules {
    destinations: BTreeMap<String, BTreeSet<HttpMethod>>,
    protected_origins: BTreeSet<String>,
}
/// Produced only by the signed policy. It contains no secret headers or IDs.
pub(super) struct ApprovedTarget {
    url: Url,
    method: Method,
    headers: HeaderMap,
    catalog: Option<Arc<AppCatalog>>,
}
impl AppHttpPolicy {
    pub(super) fn compile(package: &VerifiedPackage<'_>, catalog: Arc<AppCatalog>) -> Result<Self> {
        if package.package_digest() != catalog.package_digest() {
            return Err(HttpError::Provenance);
        }
        let rules = Rules {
            destinations: package
                .manifest()
                .capabilities
                .network
                .iter()
                .map(|entry| {
                    (
                        entry.origin.clone(),
                        entry.methods.iter().copied().collect(),
                    )
                })
                .collect(),
            protected_origins: package
                .declarations()
                .actions
                .iter()
                .filter(|action| action.critical_validation.is_some())
                .flat_map(|action| {
                    action
                        .effect_routes
                        .iter()
                        .map(|route| route.origin.clone())
                })
                .collect(),
        };
        Ok(Self { catalog, rules })
    }
    pub(super) fn catalog(&self) -> &Arc<AppCatalog> {
        &self.catalog
    }
    pub(super) fn require_current(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        trusted_owner: &str,
    ) -> Result<()> {
        self.catalog
            .require_current(transaction, trusted_owner)
            .map_err(|_| HttpError::Provenance)
    }
    pub(super) fn anonymous_target(
        &self,
        url: &str,
        method: &str,
        headers: &[(String, String)],
        connection_id: Option<&str>,
        operation_id: Option<&str>,
    ) -> Result<ApprovedTarget> {
        if connection_id.is_some() || operation_id.is_some() {
            return Err(HttpError::ConnectionAuthority);
        }
        let mut target = self.rules.target(url, method, headers)?;
        target.catalog = Some(self.catalog.clone());
        Ok(target)
    }
}
impl Rules {
    fn target(
        &self,
        raw: &str,
        method: &str,
        headers: &[(String, String)],
    ) -> Result<ApprovedTarget> {
        if raw.is_empty() || raw.len() > MAX_URL_BYTES || raw.chars().any(char::is_control) {
            return Err(HttpError::Invalid);
        }
        let url = Url::parse(raw).map_err(|_| HttpError::Invalid)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(HttpError::Invalid);
        }
        let (declared_method, method) = declared_method(method)?;
        let origin = url.origin().ascii_serialization();
        if !self
            .destinations
            .get(&origin)
            .is_some_and(|methods| methods.contains(&declared_method))
        {
            return Err(HttpError::Destination);
        }
        // Until the scoped connection/receipt executor exists, do not expose
        // alternate unauthenticated URLs on a protected service. String path
        // matching cannot establish how a service interprets encoded aliases or
        // method overrides. This conservative fence is explicit, never bypassed
        // by supplying an App-chosen operation/connection ID.
        if self.protected_origins.contains(&origin) {
            return Err(HttpError::ProtectedEffect);
        }
        Ok(ApprovedTarget {
            url,
            method,
            headers: request_headers(headers)?,
            catalog: None,
        })
    }
}
impl ApprovedTarget {
    pub(super) fn require_catalog(&self, catalog: &Arc<AppCatalog>) -> Result<()> {
        if self
            .catalog
            .as_ref()
            .is_some_and(|retained| Arc::ptr_eq(retained, catalog))
        {
            Ok(())
        } else {
            Err(HttpError::Provenance)
        }
    }
    pub(super) fn url(&self) -> &Url {
        &self.url
    }
    pub(super) fn method(&self) -> &Method {
        &self.method
    }
    pub(super) fn headers(&self) -> &HeaderMap {
        &self.headers
    }
    pub(super) fn port(&self) -> u16 {
        self.url
            .port_or_known_default()
            .expect("approved HTTP origin")
    }
    pub(super) fn validate_addresses(
        &self,
        addresses: impl IntoIterator<Item = SocketAddr>,
    ) -> Result<Vec<SocketAddr>> {
        let mut result = Vec::new();
        for address in addresses {
            if result.len() == MAX_RESOLVED_ADDRESSES {
                return Err(HttpError::Limit);
            }
            if address.port() != self.port() || !public(address) {
                return Err(HttpError::Destination);
            }
            result.push(address);
        }
        if result.is_empty() {
            return Err(HttpError::Network);
        }
        result.sort_unstable();
        result.dedup();
        Ok(result)
    }
    /// Check the actual socket before TLS or HTTP writes, not a response's
    /// remote address after the effect has already reached the destination.
    pub(super) fn require_connected(&self, selected: SocketAddr, actual: SocketAddr) -> Result<()> {
        if selected != actual || actual.port() != self.port() || !public(actual) {
            return Err(HttpError::Destination);
        }
        Ok(())
    }
}
fn public(address: SocketAddr) -> bool {
    crate::runtime::aegs_network_policy::is_globally_reachable_aegs_destination(address.ip())
}
fn declared_method(value: &str) -> Result<(HttpMethod, Method)> {
    Ok(match value {
        "GET" => (HttpMethod::Get, Method::GET),
        "HEAD" => (HttpMethod::Head, Method::HEAD),
        "POST" => (HttpMethod::Post, Method::POST),
        "PUT" => (HttpMethod::Put, Method::PUT),
        "PATCH" => (HttpMethod::Patch, Method::PATCH),
        "DELETE" => (HttpMethod::Delete, Method::DELETE),
        "OPTIONS" => (HttpMethod::Options, Method::OPTIONS),
        _ => return Err(HttpError::Invalid),
    })
}
fn request_headers(entries: &[(String, String)]) -> Result<HeaderMap> {
    if entries.len() > MAX_HEADERS {
        return Err(HttpError::Limit);
    }
    let mut headers = HeaderMap::new();
    let mut bytes = 0usize;
    for (name, value) in entries {
        bytes = bytes
            .checked_add(name.len())
            .and_then(|bytes| bytes.checked_add(value.len()))
            .ok_or(HttpError::Limit)?;
        if bytes > MAX_HEADER_BYTES {
            return Err(HttpError::Limit);
        }
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| HttpError::Invalid)?;
        if matches!(
            name.as_str(),
            "authorization"
                | "cookie"
                | "set-cookie"
                | "host"
                | "connection"
                | "content-length"
                | "transfer-encoding"
                | "te"
                | "trailer"
                | "upgrade"
                | "expect"
                | "proxy-authorization"
                | "proxy-authenticate"
                | "accept-encoding"
                | "x-http-method-override"
                | "x-method-override"
                | "x-http-method"
        ) || name.as_str().starts_with("proxy-")
            || name.as_str().starts_with("sec-")
        {
            return Err(HttpError::Invalid);
        }
        let value = HeaderValue::from_str(value).map_err(|_| HttpError::Invalid)?;
        headers.append(name, value);
    }
    Ok(headers)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) fn fixture_target() -> ApprovedTarget {
    Rules {
        destinations: BTreeMap::from([(
            "http://api.example.com".into(),
            BTreeSet::from([HttpMethod::Post]),
        )]),
        protected_origins: BTreeSet::new(),
    }
    .target(
        "http://api.example.com/fixture",
        "POST",
        &[(
            "content-type".into(),
            "multipart/form-data; boundary=fixture".into(),
        )],
    )
    .unwrap()
}
