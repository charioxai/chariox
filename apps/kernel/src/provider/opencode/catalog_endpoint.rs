use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::process::{ExitStatus, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use super::ports::resolve_opencode_catalog_port;
use crate::error::DaemonError;
use crate::provider::account_endpoints::{AccountEndpoints, ManagedAccountEndpoint};
use crate::provider::startup_diagnostic::ProviderStartupDiagnostic;
use crate::provider::OpenCodeClient;

pub(crate) const OPENCODE_CATALOG_START: &str = "start OpenCode catalog";

pub(in crate::provider) struct OpenCodeCatalogStartup {
    executable: String,
    port: u16,
}

impl OpenCodeCatalogStartup {
    pub(in crate::provider) fn failure(
        self,
        status: Option<ExitStatus>,
        diagnostic: &ProviderStartupDiagnostic,
    ) -> DaemonError {
        let (bytes, stderr, address_in_use, recognized) = diagnostic.read().map_or_else(
            || (0, String::new(), false, None),
            |output| {
                (
                    output.bytes,
                    output.opencode_stderr_excerpt(),
                    output.addresses_conflict(),
                    output.recognized_summary(),
                )
            },
        );
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, self.port));
        // OpenCode can report a generic ServeError for a failed bind.
        let address_in_use = address_in_use
            || (stderr.lines().any(|line| line == "ServeError")
                && TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok());
        crate::logging::warn_with_fields(
            "opencode.catalog",
            "OpenCode catalog helper failed to start",
            serde_json::json!({
                "provider": "opencode",
                "version": self.version(),
                "port": self.port,
                "exit_status": status.map(|status| status.to_string()),
                "diagnostic_class": if address_in_use { "address_in_use" } else if recognized.is_some() { "recognized" } else { "unclassified" },
                "diagnostic_summary": recognized,
                "stderr_bytes": bytes,
                "stderr_excerpt": stderr,
            }),
        );
        let message = if address_in_use {
            format!(
                "Another program is using port {}. Retry to start OpenCode on a free port.",
                self.port
            )
        } else if let Some(summary) = recognized {
            // A recognized boundary failure (for example a denied namespace) keeps the recovery
            // guidance: retrying cannot repair that environment setting.
            format!("OpenCode could not start its model catalog; {summary}")
        } else {
            "OpenCode could not start its model catalog. Retry starting OpenCode.".into()
        };
        DaemonError::LocalTransport {
            operation: OPENCODE_CATALOG_START,
            message,
        }
    }

    fn version(&self) -> Option<String> {
        let command = crate::provider::managed_isolated_utility_command(
            self.executable.clone(),
            vec!["--version".into()],
            BTreeMap::new(),
            None,
            "opencode:version",
        )
        .ok()?;
        let output =
            crate::runtime::bounded_process::capture::capture(command, Duration::from_secs(2))
                .ok()?;
        let version = std::str::from_utf8(&output.stdout).ok()?.trim();
        (output.status.success()
            && version.len() <= 32
            && version.split('.').count() == 3
            && version
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())))
        .then(|| version.to_string())
    }
}

static OPENCODE_ACCOUNT_ENDPOINTS: OnceLock<AccountEndpoints> = OnceLock::new();

pub fn opencode_catalog_endpoint() -> Result<String, DaemonError> {
    let port = resolve_opencode_catalog_port()?;
    Ok(format!("http://127.0.0.1:{port}"))
}

pub(crate) fn ensure_opencode_account_endpoint(
    owner_user_id: &str,
    account_profile: &str,
    environment: BTreeMap<String, String>,
) -> Result<String, DaemonError> {
    let key = format!("{owner_user_id}\0{account_profile}");
    let owner_user_id = owner_user_id.to_string();
    let account_profile = account_profile.to_string();
    OPENCODE_ACCOUNT_ENDPOINTS
        .get_or_init(|| AccountEndpoints::new("opencode.catalog"))
        .ensure(
            key,
            endpoint_is_healthy,
            Duration::from_secs(10),
            move || {
                let request = crate::provider::LaunchProviderRequest::new(
                    "provider-account",
                    "opencode",
                    "opencode",
                    &account_profile,
                    "default",
                )
                .with_owner_user_id(&owner_user_id)
                .with_provider_account_env(environment);
                let launch = super::plan_opencode_launch(Some(&request))?;
                let endpoint = launch.structured_endpoint.clone().ok_or_else(|| {
                    DaemonError::LocalTransport {
                        operation: "ensure_opencode_account_endpoint",
                        message: "OpenCode account launch did not expose an endpoint".to_string(),
                    }
                })?;
                let startup = OpenCodeCatalogStartup {
                    executable: launch
                        .pty_program
                        .clone()
                        .expect("managed OpenCode executable"),
                    port: endpoint
                        .rsplit_once(':')
                        .and_then(|(_, port)| port.parse().ok())
                        .expect("managed OpenCode port"),
                };
                let launch = crate::provider::apply_managed_provider_isolation(launch, &request)?;
                let mut command = crate::provider::command_from_provider_launch(launch)?;
                command
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped());
                for name in crate::account_profile::provider_auth_env_vars("opencode") {
                    command.env_remove(name);
                }
                #[cfg(unix)]
                unsafe {
                    use std::os::unix::process::CommandExt;
                    command.pre_exec(|| {
                        if libc::setsid() < 0 {
                            Err(std::io::Error::last_os_error())
                        } else {
                            Ok(())
                        }
                    });
                }
                let child = crate::process_spawn::spawn_command(command).map_err(|error| {
                    DaemonError::LocalTransport {
                        operation: "ensure_opencode_account_endpoint",
                        message: format!(
                            "failed to start profile-specific OpenCode server: {error}"
                        ),
                    }
                })?;
                let mut endpoint = ManagedAccountEndpoint::new(endpoint, child);
                endpoint.opencode_startup = Some(startup);
                Ok(endpoint)
            },
        )
}

pub(crate) fn invalidate_opencode_account_endpoint(owner_user_id: &str, account_profile: &str) {
    if let Some(endpoints) = OPENCODE_ACCOUNT_ENDPOINTS.get() {
        endpoints.invalidate(format!("{owner_user_id}\0{account_profile}"));
    }
}

pub(crate) fn shutdown_opencode_account_endpoints() {
    if let Some(endpoints) = OPENCODE_ACCOUNT_ENDPOINTS.get() {
        endpoints.shutdown();
    }
}

fn endpoint_is_healthy(base_url: &str) -> bool {
    OpenCodeClient::new("catalog", base_url)
        .and_then(|client| client.check_health())
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed_start(stderr: &str) -> DaemonError {
        let mut child = crate::process_spawn::command("sh")
            .args(["-c", &format!("printf '%s\\n' '{stderr}' >&2; exit 1")])
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let diagnostic = ProviderStartupDiagnostic::capture(&mut child, true);
        let status = child.wait().unwrap();
        OpenCodeCatalogStartup {
            executable: "opencode-test-missing".to_string(),
            port: 1,
        }
        .failure(Some(status), &diagnostic)
    }

    fn message(error: DaemonError) -> String {
        match error {
            DaemonError::LocalTransport { operation, message } => {
                assert_eq!(operation, OPENCODE_CATALOG_START);
                message
            }
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn a_namespace_denial_keeps_its_recovery_guidance() {
        let message = message(failed_start(
            "bwrap: Creating new namespace failed: Operation not permitted: private-fixture-marker",
        ));
        assert!(
            message.contains("slices.linux.allow_provider_sandbox_compatibility"),
            "{message}"
        );
        assert!(message.contains("recreate the slice"), "{message}");
        assert!(message.contains("namespace,permission_denied"), "{message}");
        assert!(!message.contains("private-fixture-marker"), "{message}");
        assert!(!message.contains("Retry starting OpenCode"), "{message}");
    }

    #[test]
    fn a_port_conflict_keeps_its_specialised_message() {
        let message = message(failed_start("Address already in use"));
        assert_eq!(
            message,
            "Another program is using port 1. Retry to start OpenCode on a free port."
        );
    }

    #[test]
    fn an_unrecognised_failure_says_retry_and_shows_no_provider_text() {
        let message = message(failed_start("client_secret=private-fixture-marker"));
        assert_eq!(
            message,
            "OpenCode could not start its model catalog. Retry starting OpenCode."
        );
    }
}
