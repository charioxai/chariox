use std::collections::BTreeSet;

use semver::Version;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{archive::validate_path, ErrorCode, Limits, PackageError, Result};

pub const APP_SCHEMA: &str = "chariox.app.v1";
pub const APP_CONTRACT_VERSION: u32 = 1;
pub const RESOURCE_POLICY: &str = "chariox.app.resources.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub schema: String,
    pub app_id: String,
    pub version: String,
    pub publisher: Publisher,
    pub sdk_version: String,
    pub app_contract_version: u32,
    pub min_kernel_protocol: u32,
    pub resource_policy: String,
    pub runtime: Runtime,
    pub ui: Ui,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<String>,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrations: Option<Migrations>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Publisher {
    pub id: String,
    pub key_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Runtime {
    pub engine: RuntimeEngine,
    pub entry: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeEngine {
    Node,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ui {
    pub entry: String,
    /// Where Chariox draws the private agent panel beside this App's view by
    /// default (kernel protocol 380). Omitted: at the right. The page can ask
    /// for another placement at runtime; the user's own choice wins.
    #[serde(
        default,
        rename = "agentPanel",
        skip_serializing_if = "Option::is_none"
    )]
    pub agent_panel: Option<AgentPanel>,
}

/// First kernel protocol that reads `ui.agentPanel`.
pub const AGENT_PANEL_PROTOCOL: u32 = 380;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPanel {
    pub placement: PanelPlacement,
    /// CSS pixels: the panel's width at the right or height at the bottom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelPlacement {
    Right,
    Bottom,
    /// The App shows no agent panel; approvals still reach the user.
    None,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default)]
    pub network: Vec<NetworkDestination>,
    #[serde(default)]
    pub clipboard: Vec<ClipboardAccess>,
    #[serde(default)]
    pub external_files: Vec<ExternalFileAccess>,
    #[serde(default)]
    pub workflows: Vec<AssetAccess>,
    #[serde(default)]
    pub agents: Vec<AssetAccess>,
    /// Actions the App may take through an event generator connection the
    /// owner grants it (kernel protocol 359). Omitted when empty, so older
    /// manifests and their capability summaries stay byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ConnectionAccess>,
    /// Inbox routes the App may manage for itself inside one connection its
    /// owner grants (kernel protocol 1011). Only the App world 1.1.0 has the
    /// import that uses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_routing: Option<EventRouting>,
}

/// One event generator (for example `dev.chariox.github`) whose events the App
/// routes to its own declared incoming events. The owner still chooses the
/// connection; the kernel keeps the App inside it and inside `max_routes`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventRouting {
    pub generator: String,
    /// The App's incoming events a route may deliver to.
    pub incoming: Vec<String>,
    pub max_routes: u32,
}

/// Routes one installation may hold, whatever its manifest asks for.
pub const MAX_EVENT_ROUTES: u32 = 200;
const MAX_ROUTED_EVENTS: usize = 32;

/// One event generator (for example `dev.chariox.slack`) and the actions of
/// its reviewed allow-list the App may ask for. The owner still chooses which
/// of their connections, if any, the App may use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionAccess {
    pub generator: String,
    pub actions: Vec<String>,
}

/// Destinations are exact HTTP(S) origins. Paths, redirects, connected IPs and
/// credential authority are enforced by the kernel broker at request time.
///
/// A destination with a `credential` is reached only through its `routes`: the
/// owner binds an account to the credential label, and the kernel adds that
/// account's Vault value to the request. The App never holds the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkDestination {
    pub origin: String,
    pub methods: Vec<HttpMethod>,
    /// The label the owner's account binding answers to (for example `github`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<CredentialRoute>,
}

/// One REST call a credentialed destination allows: the method, a path
/// template (`{name}` stands for one path segment) and how much authority it
/// needs. An owner binding that allows less than `safety` refuses the call.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialRoute {
    pub method: HttpMethod,
    pub path: String,
    pub safety: RouteSafety,
    /// Host suffixes (`*.blob.core.windows.net`) a read of this route may be
    /// redirected to, for an answer GitHub keeps on other storage (a job log).
    /// Nothing else is followed, and the account's credential is never sent
    /// to the redirected host.
    #[serde(
        default,
        rename = "redirectHosts",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub redirect_hosts: Vec<String>,
}

/// Ordered: a binding that allows `Write` also allows `Read`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteSafety {
    Read,
    Write,
    Destructive,
}

/// Routes one destination may declare.
const MAX_CREDENTIAL_ROUTES: usize = 128;
/// Redirect host suffixes one route may declare.
const MAX_REDIRECT_HOSTS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
    Options,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardAccess {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalFileAccess {
    UserSelected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetAccess {
    Select,
    Read,
    Create,
    Update,
    Activate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Migrations {
    pub directory: String,
    pub target_version: u32,
    /// A complete linear migration chain from a clean schema (version zero).
    pub steps: Vec<Migration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Migration {
    pub from: u32,
    pub to: u32,
    pub entry: String,
}

impl Publisher {
    pub(crate) fn validate(&self) -> Result<()> {
        if !valid_identifier(&self.id)
            || !valid_identifier(&self.key_id)
            || self.name.trim().is_empty()
            || self.name.len() > 256
        {
            return Err(invalid_manifest(
                "publisher identity, keyId and name are required",
            ));
        }
        Ok(())
    }
}

impl AgentPanel {
    pub(crate) fn size_valid(&self) -> bool {
        match (self.placement, self.size) {
            (PanelPlacement::None, Some(_)) => false,
            (_, Some(size)) => (120..=1200).contains(&size),
            (_, None) => true,
        }
    }
}

impl Capabilities {
    pub(crate) fn validate(&self) -> Result<()> {
        unique(&self.clipboard)?;
        unique(&self.external_files)?;
        unique(&self.workflows)?;
        unique(&self.agents)?;
        if self.connections.len() > 8 {
            return Err(invalid_manifest("too many connection declarations"));
        }
        let mut generators = BTreeSet::new();
        for connection in &self.connections {
            let name = |value: &str, dot: bool| {
                !value.is_empty()
                    && value.len() <= 128
                    && value.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || byte == b'-'
                            || byte == b'_'
                            || (dot && byte == b'.')
                    })
            };
            if !name(&connection.generator, true)
                || !generators.insert(&connection.generator)
                || connection.actions.is_empty()
                || connection.actions.len() > 32
                || !connection.actions.iter().all(|action| name(action, true))
            {
                return Err(invalid_manifest(
                    "connections must name distinct generators and their allowed actions",
                ));
            }
            unique(&connection.actions)?;
        }
        if let Some(routing) = &self.event_routing {
            routing.validate()?;
        }
        if self.network.len() > 64 {
            return Err(invalid_manifest("too many network destinations"));
        }
        let mut origins = BTreeSet::new();
        for destination in &self.network {
            validate_origin(&destination.origin)?;
            if !origins.insert(&destination.origin) || destination.methods.is_empty() {
                return Err(invalid_manifest(
                    "destinations must be unique and declare allowed methods",
                ));
            }
            unique(&destination.methods)?;
            destination.validate_credential()?;
        }
        Ok(())
    }
}

impl Migrations {
    /// `entry_ok` is the schema's rule for what a migration step file is:
    /// JavaScript in v1, a component in v2.
    pub(crate) fn validate(&self, limits: &Limits, entry_ok: fn(&str) -> bool) -> Result<()> {
        validate_path(&self.directory, limits)?;
        if self.directory != "migrations"
            || self.steps.is_empty()
            || self.steps.len() > limits.max_declarations
            || self.steps.len() != self.target_version as usize
        {
            return Err(invalid_manifest(
                "migrations must declare the complete bounded version chain",
            ));
        }
        let mut entries = BTreeSet::new();
        for (index, step) in self.steps.iter().enumerate() {
            validate_path(&step.entry, limits)?;
            if step.from as usize != index
                || step.to as usize != index + 1
                || !step.entry.starts_with("migrations/")
                || !entry_ok(&step.entry)
                || !entries.insert(&step.entry)
            {
                return Err(invalid_manifest(
                    "migration steps must be unique entries ordered from zero to target",
                ));
            }
        }
        Ok(())
    }
}

impl EventRouting {
    fn validate(&self) -> Result<()> {
        let generator = !self.generator.is_empty()
            && self.generator.len() <= 128
            && self.generator.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b'_' | b'.')
            });
        if !generator
            || self.incoming.is_empty()
            || self.incoming.len() > MAX_ROUTED_EVENTS
            || !self.incoming.iter().all(|name| valid_function_name(name))
            || !(1..=MAX_EVENT_ROUTES).contains(&self.max_routes)
        {
            return Err(invalid_manifest(
                "eventRouting needs a generator, its incoming events and 1 to 200 routes",
            ));
        }
        unique(&self.incoming)
    }
}

impl NetworkDestination {
    fn validate_credential(&self) -> Result<()> {
        let Some(credential) = &self.credential else {
            return if self.routes.is_empty() {
                Ok(())
            } else {
                Err(invalid_manifest("routes need a credential label"))
            };
        };
        let label = !credential.is_empty()
            && credential.len() <= 64
            && credential
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !label
            || !self.origin.starts_with("https://")
            || self.routes.is_empty()
            || self.routes.len() > MAX_CREDENTIAL_ROUTES
        {
            return Err(invalid_manifest(
                "a credential needs a label, an https origin and bounded routes",
            ));
        }
        let mut seen = BTreeSet::new();
        for route in &self.routes {
            if !route_path(&route.path) || !seen.insert((route.method, &route.path)) {
                return Err(invalid_manifest(
                    "credential routes must be unique, exact path templates",
                ));
            }
            if !route.redirect_hosts.is_empty()
                && (route.method != HttpMethod::Get
                    || route.safety != RouteSafety::Read
                    || route.redirect_hosts.len() > MAX_REDIRECT_HOSTS
                    || !route.redirect_hosts.iter().all(|host| redirect_host(host)))
            {
                return Err(invalid_manifest(
                    "redirectHosts belong to a read route and are host suffixes like *.example.com",
                ));
            }
        }
        let methods: BTreeSet<_> = self.routes.iter().map(|route| route.method).collect();
        if methods != self.methods.iter().copied().collect() {
            return Err(invalid_manifest(
                "a credentialed destination's methods must be the methods of its routes",
            ));
        }
        Ok(())
    }
}

/// `*.example.com`: a wildcard label and a lowercase DNS suffix of at least
/// two labels, so a redirect can never be allowed to a whole top-level domain.
fn redirect_host(pattern: &str) -> bool {
    let Some(suffix) = pattern.strip_prefix("*.") else {
        return false;
    };
    suffix.len() <= 253
        && suffix.split('.').count() >= 2
        && suffix.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// `/a/{b}/c`: segments are literal or one `{name}`; no query, fragment,
/// encoding, empty or dot segments, so a route names exactly one path shape.
fn route_path(path: &str) -> bool {
    path.len() <= 256
        && path.starts_with('/')
        && path[1..].split('/').all(|segment| {
            let literal = |text: &str| {
                !text.is_empty()
                    && text != "."
                    && text != ".."
                    && text
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
            };
            segment
                .strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))
                .map_or_else(
                    || literal(segment),
                    |inner| {
                        !inner.is_empty()
                            && inner
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    },
                )
        })
}

fn invalid_manifest(message: &'static str) -> PackageError {
    PackageError::new(ErrorCode::InvalidManifest, message)
}

impl Manifest {
    pub fn validate(&self, limits: &Limits) -> Result<()> {
        let invalid = |message| PackageError::new(ErrorCode::InvalidManifest, message);
        if self.schema != APP_SCHEMA {
            return Err(invalid("unsupported manifest schema"));
        }
        if !valid_app_id(&self.app_id) {
            return Err(invalid("appId must be a lowercase reverse-DNS identifier"));
        }
        Version::parse(&self.version).map_err(|_| invalid("version must be SemVer"))?;
        Version::parse(&self.sdk_version).map_err(|_| invalid("sdkVersion must be SemVer"))?;
        if self.app_contract_version == 0
            || self.min_kernel_protocol == 0
            || self.resource_policy.is_empty()
        {
            return Err(invalid(
                "contract, protocol and resource policy are required",
            ));
        }
        self.publisher.validate()?;
        validate_path(&self.runtime.entry, limits)?;
        validate_path(&self.ui.entry, limits)?;
        if !self.runtime.entry.starts_with("runtime/") || !javascript_path(&self.runtime.entry) {
            return Err(invalid("runtime entry must be JavaScript inside runtime/"));
        }
        if !self.ui.entry.starts_with("ui/") || !self.ui.entry.ends_with(".html") {
            return Err(invalid("UI entry must be HTML inside ui/"));
        }
        if let Some(panel) = &self.ui.agent_panel {
            if !panel.size_valid() || self.min_kernel_protocol < AGENT_PANEL_PROTOCOL {
                return Err(invalid(
                    "ui.agentPanel needs minKernelProtocol 380 and a size of 120 to 1200 CSS pixels (none takes no size)",
                ));
            }
        }
        for path in [&self.tools, &self.events, &self.actions]
            .into_iter()
            .flatten()
        {
            validate_path(path, limits)?;
            if !path.starts_with("schemas/") || !path.ends_with(".json") {
                return Err(invalid("declarations must be JSON inside schemas/"));
            }
        }
        self.capabilities.validate()?;
        if self.capabilities.event_routing.is_some() {
            return Err(invalid("eventRouting needs a v2 App"));
        }
        if let Some(migrations) = &self.migrations {
            migrations.validate(limits, javascript_path)?;
        }
        Ok(())
    }

    pub(crate) fn required_paths(&self) -> Vec<&str> {
        let mut paths = vec![self.runtime.entry.as_str(), self.ui.entry.as_str()];
        paths.extend(
            [&self.tools, &self.events, &self.actions]
                .into_iter()
                .flatten()
                .map(String::as_str),
        );
        if let Some(migrations) = &self.migrations {
            paths.extend(migrations.steps.iter().map(|step| step.entry.as_str()));
        }
        paths
    }
}

pub(crate) fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        && value
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
}

pub(crate) fn valid_function_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        && value.as_bytes()[0].is_ascii_lowercase()
}

pub(crate) fn valid_app_id(value: &str) -> bool {
    value.len() <= 128
        && value.split('.').count() >= 3
        && value.split('.').all(|part| {
            !part.is_empty()
                && part.as_bytes()[0].is_ascii_lowercase()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                && !part.ends_with('-')
        })
}

pub(crate) fn javascript_path(path: &str) -> bool {
    [".js", ".mjs", ".cjs"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
}

pub(crate) fn validate_origin(origin: &str) -> Result<()> {
    let parsed = Url::parse(origin)
        .map_err(|_| PackageError::new(ErrorCode::InvalidManifest, "invalid network origin"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || parsed.origin().ascii_serialization() != origin
    {
        return Err(PackageError::new(ErrorCode::InvalidManifest, "network destinations must be canonical exact HTTP(S) origins without paths or credentials"));
    }
    Ok(())
}

fn unique<T: Ord>(values: &[T]) -> Result<()> {
    if values.iter().collect::<BTreeSet<_>>().len() != values.len() {
        return Err(PackageError::new(
            ErrorCode::InvalidManifest,
            "duplicate capability",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod credential_tests {
    use super::*;

    fn destination(routes: &[(HttpMethod, &str, RouteSafety)]) -> NetworkDestination {
        NetworkDestination {
            origin: "https://api.github.com".into(),
            methods: routes
                .iter()
                .map(|route| route.0)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            credential: Some("github".into()),
            routes: routes
                .iter()
                .map(|(method, path, safety)| CredentialRoute {
                    method: *method,
                    path: (*path).into(),
                    safety: *safety,
                    redirect_hosts: Vec::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn a_credentialed_destination_is_a_bounded_set_of_exact_routes() {
        let read = (
            HttpMethod::Get,
            "/repos/{owner}/{repo}/pulls/{number}",
            RouteSafety::Read,
        );
        let merge = (
            HttpMethod::Put,
            "/repos/{owner}/{repo}/pulls/{number}/merge",
            RouteSafety::Destructive,
        );
        destination(&[read, merge]).validate_credential().unwrap();
        assert!(
            RouteSafety::Read < RouteSafety::Write && RouteSafety::Write < RouteSafety::Destructive
        );

        let refused = [
            // Routes without a credential label.
            NetworkDestination {
                credential: None,
                ..destination(&[read])
            },
            // A bearer token over plain HTTP.
            NetworkDestination {
                origin: "http://api.github.com".into(),
                ..destination(&[read])
            },
            // Methods that no route uses.
            NetworkDestination {
                methods: vec![HttpMethod::Get, HttpMethod::Delete],
                ..destination(&[read])
            },
            // A credential with no route at all.
            NetworkDestination {
                routes: Vec::new(),
                ..destination(&[read])
            },
            destination(&[read, read]),
            destination(&[(HttpMethod::Get, "/repos/%2e%2e", RouteSafety::Read)]),
            destination(&[(HttpMethod::Get, "/repos/../x", RouteSafety::Read)]),
            destination(&[(HttpMethod::Get, "/repos?x=1", RouteSafety::Read)]),
            destination(&[(HttpMethod::Get, "/repos/{}", RouteSafety::Read)]),
            destination(&[(HttpMethod::Get, "/repos//x", RouteSafety::Read)]),
            destination(&[(HttpMethod::Get, "repos", RouteSafety::Read)]),
        ];
        for destination in refused {
            assert!(
                destination.validate_credential().is_err(),
                "{destination:?}"
            );
        }
    }

    #[test]
    fn a_read_route_may_name_the_host_suffixes_its_answer_is_redirected_to_and_no_others_may() {
        let logs = "/repos/{owner}/{repo}/actions/jobs/{job_id}/logs";
        let with = |method, safety, hosts: &[&str]| {
            let mut destination = destination(&[(method, logs, safety)]);
            destination.routes[0].redirect_hosts = hosts.iter().map(|h| (*h).into()).collect();
            destination
        };
        with(
            HttpMethod::Get,
            RouteSafety::Read,
            &["*.blob.core.windows.net"],
        )
        .validate_credential()
        .unwrap();
        for hosts in [
            &["blob.core.windows.net"][..],
            &["*.com"],
            &["*"],
            &["*.Blob.core.windows.net"],
            &["*.-x.example.com"],
            &[
                "*.a.example.com",
                "*.b.example.com",
                "*.c.example.com",
                "*.d.example.com",
                "*.e.example.com",
            ],
        ] {
            assert!(
                with(HttpMethod::Get, RouteSafety::Read, hosts)
                    .validate_credential()
                    .is_err(),
                "{hosts:?}"
            );
        }
        // A redirect belongs to a read: a write or a destructive route follows none.
        assert!(with(
            HttpMethod::Post,
            RouteSafety::Write,
            &["*.blob.core.windows.net"]
        )
        .validate_credential()
        .is_err());
        assert!(with(
            HttpMethod::Get,
            RouteSafety::Write,
            &["*.blob.core.windows.net"]
        )
        .validate_credential()
        .is_err());
    }
}

#[cfg(test)]
mod event_routing_tests {
    use super::*;

    fn routing() -> EventRouting {
        EventRouting {
            generator: "dev.chariox.github".into(),
            incoming: vec!["pull_request".into(), "check_run".into()],
            max_routes: 200,
        }
    }

    fn capabilities(routing: EventRouting) -> Capabilities {
        Capabilities {
            event_routing: Some(routing),
            ..Capabilities::default()
        }
    }

    #[test]
    fn event_routing_names_one_generator_its_incoming_events_and_a_bounded_route_count() {
        capabilities(routing()).validate().unwrap();
        let refused = [
            EventRouting {
                generator: "Dev GitHub".into(),
                ..routing()
            },
            EventRouting {
                incoming: Vec::new(),
                ..routing()
            },
            EventRouting {
                incoming: vec!["pull_request".into(), "pull_request".into()],
                ..routing()
            },
            EventRouting {
                incoming: vec!["Pull Request".into()],
                ..routing()
            },
            EventRouting {
                incoming: (0..33).map(|n| format!("event_{n}")).collect(),
                ..routing()
            },
            EventRouting {
                max_routes: 0,
                ..routing()
            },
            EventRouting {
                max_routes: MAX_EVENT_ROUTES + 1,
                ..routing()
            },
        ];
        for routing in refused {
            assert!(
                capabilities(routing.clone()).validate().is_err(),
                "{routing:?}"
            );
        }
    }

    #[test]
    fn an_app_without_event_routing_keeps_its_manifest_bytes() {
        let json = serde_json::to_string(&Capabilities::default()).unwrap();
        assert!(!json.contains("eventRouting"), "{json}");
        let routed = serde_json::to_value(capabilities(routing())).unwrap();
        assert_eq!(routed["eventRouting"]["maxRoutes"], 200);
        assert_eq!(routed["eventRouting"]["generator"], "dev.chariox.github");
    }
}
