//! Immutable declared destinations plus the existing live installation fence.
//! A publisher declaration is insufficient by itself: require_current must run
//! on the existing writer before an admitted request can write to its socket.
use super::{HttpError, Result};
use chariox_app_package::{HttpMethod, RouteSafety, VerifiedApp};
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
use zeroize::Zeroizing;

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
    /// (origin, method, exact path) → critical action it effects.
    protected_routes: BTreeMap<(String, HttpMethod, String), String>,
    /// Destinations reached only with an owner's account, through signed routes.
    credentialed: BTreeMap<String, Credentialed>,
}
struct Credentialed {
    label: String,
    routes: Vec<Route>,
}
struct Route {
    method: HttpMethod,
    path: Vec<Segment>,
    safety: RouteSafety,
    /// Host suffixes (`*.example.com`) the answer of this route may be redirected to.
    redirect_hosts: Vec<String>,
}
enum Segment {
    Literal(String),
    /// `{name}`: any one non-empty path segment.
    Param,
}
/// How a request may reach a destination.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reach {
    Anonymous,
    Protected,
    Account,
}
/// Request headers an App may set on an account's call. Everything the
/// kernel adds (`authorization`, `user-agent`, `host`) is not in the list.
const ACCOUNT_REQUEST_HEADERS: [&str; 6] = [
    "accept",
    "content-type",
    "if-match",
    "if-none-match",
    "if-modified-since",
    "x-github-api-version",
];
/// What an account's call is for, before its Vault value is read.
pub(super) struct AccountCall {
    pub(super) target: ApprovedTarget,
    pub(super) credential: String,
    pub(super) origin: String,
    pub(super) safety: RouteSafety,
}
/// Produced only by the signed policy. It contains no secret headers or IDs.
pub(super) struct ApprovedTarget {
    url: Url,
    method: Method,
    headers: HeaderMap,
    catalog: Option<Arc<AppCatalog>>,
    /// Only a protected effect's approved parameters; the App adds no body.
    body: Option<bytes::Bytes>,
    /// The account's `Authorization` value, added by the kernel just before
    /// the request is written. Its presence also marks the answer for
    /// scrubbing, header filtering and the response cap.
    authorization: Option<Zeroizing<String>>,
    /// Where a redirect of the answer may go (the matched route's signed host
    /// suffixes). Empty: a redirect is the answer.
    redirect_hosts: Vec<String>,
    /// Whether the account's `Authorization` is written to this host. It is
    /// not on a redirected host, which still has its answer scrubbed.
    sends_authorization: bool,
}
impl AppHttpPolicy {
    pub(super) fn compile(package: &impl VerifiedApp, catalog: Arc<AppCatalog>) -> Result<Self> {
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
            credentialed: package
                .manifest()
                .capabilities
                .network
                .iter()
                .filter_map(|entry| {
                    let label = entry.credential.clone()?;
                    let routes = entry
                        .routes
                        .iter()
                        .map(|route| Route {
                            method: route.method,
                            path: route.path[1..]
                                .split('/')
                                .map(|segment| {
                                    if segment.starts_with('{') {
                                        Segment::Param
                                    } else {
                                        Segment::Literal(segment.to_owned())
                                    }
                                })
                                .collect(),
                            safety: route.safety,
                            redirect_hosts: route.redirect_hosts.clone(),
                        })
                        .collect();
                    Some((entry.origin.clone(), Credentialed { label, routes }))
                })
                .collect(),
            protected_routes: package
                .declarations()
                .actions
                .iter()
                .filter(|action| action.critical_validation.is_some())
                .flat_map(|action| {
                    action.effect_routes.iter().map(|route| {
                        (
                            (route.origin.clone(), route.method, route.path.clone()),
                            action.name.clone(),
                        )
                    })
                })
                .collect(),
        };
        Ok(Self { catalog, rules })
    }
    pub(super) fn catalog(&self) -> &Arc<AppCatalog> {
        &self.catalog
    }
    pub(super) fn anonymous_target(
        &self,
        url: &str,
        method: &str,
        headers: &[(String, String)],
    ) -> Result<ApprovedTarget> {
        let mut target = self
            .rules
            .target_with(url, method, headers, Reach::Anonymous)?;
        target.catalog = Some(self.catalog.clone());
        Ok(target)
    }
    /// A call as the owner's account (`connection-id` `account:<label>`): it
    /// must match one route of the signed table exactly, and sets no header
    /// but the few the kernel lets through. The caller checks the owner's
    /// binding and adds the credential.
    pub(super) fn account_target(
        &self,
        url: &str,
        method: &str,
        headers: &[(String, String)],
    ) -> Result<AccountCall> {
        let mut call = self.rules.account_call(url, method, headers)?;
        call.target.catalog = Some(self.catalog.clone());
        Ok(call)
    }

    /// A request carrying a validation operation reaches a protected service
    /// only on an exact declared effect route of a critical action: no query,
    /// no percent-encoded path and no method-override header, so an alias of
    /// the route cannot be approved by accident, and no App header but
    /// `accept`. Returns the action it effects; the writer spends the matching
    /// approval before any I/O.
    pub(super) fn protected_target(
        &self,
        url: &str,
        method: &str,
        headers: &[(String, String)],
    ) -> Result<(ApprovedTarget, String)> {
        let (mut target, action) = self.rules.protected(url, method, headers)?;
        target.catalog = Some(self.catalog.clone());
        Ok((target, action))
    }
}
impl Rules {
    fn account_call(
        &self,
        url: &str,
        method: &str,
        headers: &[(String, String)],
    ) -> Result<AccountCall> {
        if headers.iter().any(|(name, _)| {
            !ACCOUNT_REQUEST_HEADERS
                .iter()
                .any(|allowed| name.eq_ignore_ascii_case(allowed))
        }) {
            return Err(HttpError::Invalid);
        }
        let target = self.target_with(url, method, headers, Reach::Account)?;
        let origin = target.url.origin().ascii_serialization();
        let destination = self
            .credentialed
            .get(&origin)
            .ok_or(HttpError::AccountUnknown)?;
        let (declared, _) = declared_method(method)?;
        let segments: Vec<&str> = target
            .url
            .path_segments()
            .map(Iterator::collect)
            .unwrap_or_default();
        // A backend may decode a percent-encoded parameter into a path of its own, like
        // `protected` refuses.
        if segments.iter().any(|segment| segment.contains('%')) {
            return Err(HttpError::RouteNotDeclared);
        }
        let route = destination
            .routes
            .iter()
            .find(|route| {
                route.method == declared
                    && route.path.len() == segments.len()
                    && route
                        .path
                        .iter()
                        .zip(&segments)
                        .all(|(want, got)| match want {
                            Segment::Literal(text) => text == got,
                            Segment::Param => !got.is_empty(),
                        })
            })
            .ok_or(HttpError::RouteNotDeclared)?;
        let mut target = target;
        target.redirect_hosts = route.redirect_hosts.clone();
        Ok(AccountCall {
            credential: destination.label.clone(),
            safety: route.safety,
            origin,
            target,
        })
    }

    fn protected(
        &self,
        raw: &str,
        method: &str,
        headers: &[(String, String)],
    ) -> Result<(ApprovedTarget, String)> {
        // The effect is the approved body on the declared route: no App-chosen
        // header can change it (method overrides are refused everywhere).
        if headers
            .iter()
            .any(|(name, _)| !name.eq_ignore_ascii_case("accept"))
        {
            return Err(HttpError::ProtectedEffect);
        }
        let target = self.target_with(raw, method, headers, Reach::Protected)?;
        let (declared, _) = declared_method(method)?;
        // Only methods that carry the approved parameters as their body.
        if !matches!(
            declared,
            HttpMethod::Post | HttpMethod::Put | HttpMethod::Patch
        ) {
            return Err(HttpError::ProtectedEffect);
        }
        let path = target.url.path().to_owned();
        if target.url.query().is_some() || path.contains('%') {
            return Err(HttpError::ProtectedEffect);
        }
        let action = self
            .protected_routes
            .get(&(target.url.origin().ascii_serialization(), declared, path))
            .cloned()
            .ok_or(HttpError::ProtectedEffect)?;
        Ok((target, action))
    }
    #[cfg(test)]
    fn target(
        &self,
        raw: &str,
        method: &str,
        headers: &[(String, String)],
    ) -> Result<ApprovedTarget> {
        self.target_with(raw, method, headers, Reach::Anonymous)
    }
    fn target_with(
        &self,
        raw: &str,
        method: &str,
        headers: &[(String, String)],
        reach: Reach,
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
        if let Some(url::Host::Domain(host)) = url.host() {
            super::dns::absolute_name(host)?;
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
        // A protected service is reached only through `protected` (an exact
        // declared effect route whose approval the writer spends). String path
        // matching cannot establish how a service interprets encoded aliases or
        // method overrides, so every other request to its origin is refused.
        if reach != Reach::Protected && self.protected_origins.contains(&origin) {
            return Err(HttpError::ProtectedEffect);
        }
        // A credentialed destination is reached only with an account, and an
        // account only reaches a credentialed destination.
        if (reach == Reach::Account) != self.credentialed.contains_key(&origin) {
            return Err(if reach == Reach::Account {
                HttpError::AccountUnknown
            } else {
                HttpError::AccountRequired
            });
        }
        Ok(ApprovedTarget {
            url,
            method,
            headers: request_headers(headers)?,
            catalog: None,
            body: None,
            authorization: None,
            redirect_hosts: Vec::new(),
            sends_authorization: true,
        })
    }
}
impl ApprovedTarget {
    pub(super) fn with_authorization(mut self, authorization: Zeroizing<String>) -> Self {
        self.authorization = Some(authorization);
        self
    }
    pub(super) fn authorization(&self) -> Option<&str> {
        self.authorization.as_ref().map(|value| value.as_str())
    }
    /// The header the request carries: the account's value, except on a redirected host.
    pub(super) fn sent_authorization(&self) -> Option<&str> {
        self.authorization().filter(|_| self.sends_authorization)
    }

    /// Where an answer's redirect goes, when the signed route allows that host:
    /// a read of an https host under one of the route's suffixes, with no
    /// credential and no further redirect. Anything else is not followed, and
    /// the redirect stays the answer.
    pub(super) fn redirected(&self, location: &str) -> Option<Self> {
        if self.redirect_hosts.is_empty() || location.len() > MAX_URL_BYTES {
            return None;
        }
        let url = self.url.join(location).ok()?;
        let Some(url::Host::Domain(host)) = url.host() else {
            return None;
        };
        let allowed = self.redirect_hosts.iter().any(|pattern| {
            pattern
                .strip_prefix('*')
                .is_some_and(|suffix| host.len() > suffix.len() && host.ends_with(suffix))
        });
        if !allowed
            || url.scheme() != "https"
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || super::dns::absolute_name(host).is_err()
        {
            return None;
        }
        Some(Self {
            url,
            method: Method::GET,
            headers: HeaderMap::new(),
            catalog: self.catalog.clone(),
            body: None,
            authorization: self.authorization.clone(),
            redirect_hosts: Vec::new(),
            sends_authorization: false,
        })
    }

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
    /// The effect sends exactly the approved canonical parameters as JSON.
    pub(super) fn with_approved_body(mut self, parameters: String) -> Self {
        self.headers.insert(
            hyper::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        self.body = Some(parameters.into());
        self
    }
    pub(super) fn take_body(&mut self) -> Option<bytes::Bytes> {
        self.body.take()
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
        protected_routes: BTreeMap::new(),
        credentialed: BTreeMap::new(),
    }
    .target_with(
        "http://api.example.com/fixture",
        "POST",
        &[(
            "content-type".into(),
            "multipart/form-data; boundary=fixture".into(),
        )],
        Reach::Anonymous,
    )
    .unwrap()
}

/// A GET target for `url`, whose origin is the only declared destination.
#[cfg(test)]
pub(super) fn fixture_get(url: &str) -> ApprovedTarget {
    let origin = Url::parse(url).unwrap().origin().ascii_serialization();
    Rules {
        destinations: BTreeMap::from([(origin, BTreeSet::from([HttpMethod::Get]))]),
        protected_origins: BTreeSet::new(),
        protected_routes: BTreeMap::new(),
        credentialed: BTreeMap::new(),
    }
    .target_with(url, "GET", &[], Reach::Anonymous)
    .unwrap()
}

/// An account's GET of `url` whose route may be redirected to `redirect_hosts`, with the
/// account's `Authorization` already attached.
#[cfg(test)]
pub(super) fn fixture_account_get(url: &str, redirect_hosts: &[&str]) -> ApprovedTarget {
    let mut target =
        fixture_get(url).with_authorization(Zeroizing::new("Bearer ghp_secret".into()));
    target.redirect_hosts = redirect_hosts.iter().map(|host| (*host).into()).collect();
    target
}

/// A policy whose only destination is the credentialed `https://api.github.com`.
#[cfg(test)]
pub(super) fn fixture_account_policy(catalog: Arc<AppCatalog>) -> AppHttpPolicy {
    AppHttpPolicy {
        catalog,
        rules: tests::github_rules(),
    }
}
