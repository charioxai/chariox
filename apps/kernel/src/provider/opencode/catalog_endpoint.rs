use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use super::ports::resolve_opencode_catalog_port;
use crate::error::DaemonError;
use crate::provider::account_endpoints::{AccountEndpoints, ManagedAccountEndpoint};
use crate::provider::OpenCodeClient;

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
                let launch = crate::provider::apply_managed_provider_isolation(
                    super::plan_opencode_launch(Some(&request))?,
                    &request,
                )?;
                let endpoint = launch.structured_endpoint.clone().ok_or_else(|| {
                    DaemonError::LocalTransport {
                        operation: "ensure_opencode_account_endpoint",
                        message: "OpenCode account launch did not expose an endpoint".to_string(),
                    }
                })?;
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
                Ok(ManagedAccountEndpoint::new(endpoint, child))
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
