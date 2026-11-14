//! Reuse the provider logins that already exist on this machine.
//!
//! Chariox does not store these credentials. It finds the installed provider
//! CLIs, asks each CLI whether the machine-level login is valid, and registers
//! the provider-native location in place. A second kernel of the same OS user
//! finds the same logins at its first start without asking the user to sign in.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
#[cfg(target_os = "macos")]
use sha2::{Digest, Sha256};

use crate::account_profile::{
    ProviderAccountAuthState, ProviderAccountProfile, ProviderAccountProfileOrigin,
    ProviderAccountProfileRegistry,
};
use crate::config::DaemonConfig;
use crate::error::DaemonError;

const PROVIDERS: [&str; 3] = ["codex", "claude", "opencode"];
const MACHINE_LOCATIONS_FILE: &str = ".config/chariox/provider-locations.json";

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct DiscoveryOutcome {
    /// Default profiles whose machine login was checked.
    pub(crate) validated: Vec<String>,
    /// Profiles added from provider-native machine locations.
    pub(crate) adopted: Vec<String>,
}

/// Provider-native roots a kernel of this OS user linked, shared by all of them.
/// It holds paths only, never credentials; the provider CLI judges each login.
#[derive(Default, Serialize, Deserialize)]
struct MachineLocations {
    #[serde(default)]
    locations: Vec<MachineLocation>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MachineLocation {
    provider: String,
    path: PathBuf,
}

fn machine_locations_path(home: &Path) -> PathBuf {
    home.join(MACHINE_LOCATIONS_FILE)
}

fn read_machine_locations(home: &Path) -> Vec<MachineLocation> {
    let path = machine_locations_path(home);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.provider_discovery",
                "machine provider locations are unreadable",
                serde_json::json!({ "error": error.to_string() }),
            );
            return Vec::new();
        }
    };
    serde_json::from_slice::<MachineLocations>(&bytes)
        .map(|file| file.locations)
        .unwrap_or_else(|error| {
            crate::logging::warn_with_fields(
                "daemon.provider_discovery",
                "machine provider locations are malformed; ignoring them",
                serde_json::json!({ "error": error.to_string() }),
            );
            Vec::new()
        })
}

/// Publish a provider-native root a user linked so other kernels can reuse it.
pub(crate) fn record_machine_location(
    home: &Path,
    kernel_private_root: &Path,
    provider: &str,
    root: &Path,
) -> Result<(), DaemonError> {
    let Ok(root) = std::fs::canonicalize(root) else {
        return Ok(());
    };
    // A login inside a kernel's own home belongs to that kernel; sharing it is an
    // explicit copy, never a link to the same files.
    if root.starts_with(kernel_private_root) {
        return Ok(());
    }
    let mut locations = read_machine_locations(home);
    let location = MachineLocation {
        provider: provider.to_string(),
        path: root,
    };
    if locations.contains(&location) {
        return Ok(());
    }
    locations.push(location);
    let bytes = serde_json::to_vec_pretty(&MachineLocations { locations }).map_err(|error| {
        DaemonError::LocalTransport {
            operation: "record machine provider location",
            message: error.to_string(),
        }
    })?;
    crate::account_profile::write_private_machine_file(&machine_locations_path(home), &bytes)
}

/// Find installed provider CLIs, validate their machine-level logins and adopt
/// logins that live in provider-native machine locations. Blocking.
pub(crate) fn discover_machine_providers(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    home: &Path,
    config: &DaemonConfig,
) -> DiscoveryOutcome {
    discover_at(
        registry,
        owner_user_id,
        home,
        config,
        crate::session::unix_epoch_ms(),
    )
}

fn discover_at(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    home: &Path,
    config: &DaemonConfig,
    now_ms: u64,
) -> DiscoveryOutcome {
    let mut outcome = DiscoveryOutcome::default();
    if crate::provider::managed_provider_isolation_required() {
        return outcome;
    }
    for provider in PROVIDERS {
        if !provider_installed(provider) {
            continue;
        }
        let profiles = match registry.list(owner_user_id, Some(provider)) {
            Ok(profiles) => profiles,
            Err(error) => {
                log_failure(provider, "list profiles", &error);
                continue;
            }
        };
        for profile in profiles
            .iter()
            .filter(|profile| needs_validation(profile, now_ms))
        {
            if validate(registry, owner_user_id, profile, config) {
                outcome.validated.push(profile.profile_id.clone());
            }
        }
        for root in candidate_roots(provider, home) {
            match registry.machine_location_due(provider, &root, now_ms) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => {
                    log_failure(provider, "check machine location", &error);
                    continue;
                }
            }
            if let Some(profile) = adopt(registry, owner_user_id, provider, &root, config, now_ms) {
                outcome.adopted.push(profile.profile_id);
            }
        }
    }
    outcome
}

fn provider_installed(provider: &str) -> bool {
    match provider {
        "codex" => crate::provider::resolve_codex_executable().is_ok(),
        "claude" => crate::provider::resolve_claude_executable().is_ok(),
        "opencode" => crate::provider::resolve_opencode_executable().is_ok(),
        _ => false,
    }
}

/// A login is rechecked at most once a minute, so asking again on every client
/// open stays cheap while an account switched in the provider's own CLI shows up
/// within a minute of the next open. Codex is never rechecked on its own: its
/// server keeps the login it saw at its start and only an explicit refresh
/// reloads it. A profile rejected as `Error` is rechecked the same way: the
/// default login owns an account's identity while it is signed in as that
/// account, so a linked profile it displaced is valid again once the default
/// switches back.
const LOGIN_RECHECK_MS: u64 = 60 * 1000;

/// Only profiles on the machine's own provider login are checked here; a profile
/// Chariox created needs its own sign-in.
fn needs_validation(profile: &ProviderAccountProfile, now_ms: u64) -> bool {
    if profile.origin == ProviderAccountProfileOrigin::CharioxCreated {
        return false;
    }
    match profile.auth_state {
        ProviderAccountAuthState::Unknown => true,
        ProviderAccountAuthState::NotConfigured
        | ProviderAccountAuthState::Expired
        | ProviderAccountAuthState::Authenticated
        | ProviderAccountAuthState::Error => {
            profile.provider != "codex"
                && profile
                    .last_validated_at_ms
                    .is_none_or(|checked| now_ms.saturating_sub(checked) >= LOGIN_RECHECK_MS)
        }
    }
}

/// True when the provider CLI answered, whatever the answer was.
fn validate(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    profile: &ProviderAccountProfile,
    config: &DaemonConfig,
) -> bool {
    // Defaults use file credentials. An absent native login cannot authenticate;
    // leave Unknown so the next discovery can find a login made in the CLI,
    // without starting a resident app-server just to learn that nobody signed in.
    if profile.provider == "codex"
        && profile.origin == ProviderAccountProfileOrigin::Default
        && profile.auth_state != ProviderAccountAuthState::Authenticated
    {
        let environment =
            match registry.resolve_environment(owner_user_id, "codex", &profile.profile_id) {
                Ok(environment) => environment,
                Err(error) => {
                    log_failure("codex", "resolve machine login location", &error);
                    return false;
                }
            };
        match Path::new(&environment["CODEX_HOME"])
            .join("auth.json")
            .try_exists()
        {
            Ok(false) => return false,
            Ok(true) => {}
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.provider_discovery",
                    "machine Codex login location is unreadable",
                    serde_json::json!({ "error": error.to_string() }),
                );
                return false;
            }
        }
    }
    // Authentication only: a usage probe that fails must not hide a valid login,
    // and a running Codex server that owns a pending device login is left alone.
    let result = crate::local::provider_requests::refresh_provider_account_auth_with_config(
        registry,
        owner_user_id,
        &profile.provider,
        &profile.profile_id,
        config,
    );
    match result {
        Ok(_) => true,
        Err(error) => {
            log_failure(&profile.provider, "validate machine login", &error);
            false
        }
    }
}

/// Register a native root as a linked profile when its login is valid and is a
/// different account from every profile already registered. Otherwise the root
/// is left as if never seen, so a login that appears there later is found.
fn adopt(
    registry: &ProviderAccountProfileRegistry,
    owner_user_id: &str,
    provider: &str,
    root: &Path,
    config: &DaemonConfig,
    now_ms: u64,
) -> Option<ProviderAccountProfile> {
    let profile = match registry.link_existing(owner_user_id, provider, "", root) {
        Ok(profile) => profile,
        Err(error) => {
            log_failure(provider, "link machine login", &error);
            return None;
        }
    };
    let validated = validate(registry, owner_user_id, &profile, config);
    let authenticated = validated
        && registry
            .get(owner_user_id, provider, &profile.profile_id)
            .is_ok_and(|profile| profile.auth_state == ProviderAccountAuthState::Authenticated);
    if authenticated {
        return registry
            .get(owner_user_id, provider, &profile.profile_id)
            .ok();
    }
    // A failed probe, a signed-out directory or a second path to an account
    // already registered.
    if let Err(error) = registry.withdraw_discovered_link(
        owner_user_id,
        provider,
        &profile.profile_id,
        root,
        now_ms,
    ) {
        log_failure(provider, "undo machine login link", &error);
    }
    None
}

fn candidate_roots(provider: &str, home: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = read_machine_locations(home)
        .into_iter()
        .filter(|location| location.provider == provider)
        .map(|location| location.path)
        .collect();
    if provider == "claude" {
        roots.extend(claude_config_dirs_with_logins(home));
    }
    roots.retain(|root| root.is_dir());
    roots
}

/// Other Claude Code logins of this user: configuration directories next to
/// `~/.claude` that hold a login. The login is only detected here, never read.
fn claude_config_dirs_with_logins(home: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(home) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(".claude") && name != ".claude" && entry.path().is_dir()
        })
        .map(|entry| entry.path())
        .filter(|dir| claude_login_present(dir))
        .collect();
    dirs.sort();
    dirs
}

#[cfg(target_os = "macos")]
fn claude_login_present(config_dir: &Path) -> bool {
    use crate::runtime::bounded_process::BoundedCommandExt as _;
    // Claude Code keeps a non-default scope's login in a Keychain item named
    // after the first 8 hex digits of the SHA-256 of the configuration path.
    // Without `-w`, `security` reports only that the item exists.
    let digest = Sha256::digest(config_dir.to_string_lossy().as_bytes());
    let service = format!(
        "Claude Code-credentials-{}",
        digest[..4]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let mut command = std::process::Command::new("/usr/bin/security");
    command.args(["find-generic-password", "-s", &service]);
    command.stdout(std::process::Stdio::null());
    command.stderr(std::process::Stdio::null());
    command
        .bounded_status()
        .is_ok_and(|status| status.success())
        || config_dir.join(".credentials.json").is_file()
}

#[cfg(not(target_os = "macos"))]
fn claude_login_present(config_dir: &Path) -> bool {
    config_dir.join(".credentials.json").is_file()
}

fn log_failure(provider: &str, step: &str, error: &DaemonError) {
    crate::logging::info_with_fields(
        "daemon.provider_discovery",
        "provider discovery step did not complete",
        serde_json::json!({ "provider": provider, "step": step, "error": error.to_string() }),
    );
}

#[cfg(test)]
mod tests;
