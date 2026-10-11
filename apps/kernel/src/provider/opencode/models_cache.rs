//! MP-08/MP-10: OpenCode reads its model catalog once at server startup. With a
//! cold models cache, as on a freshly materialized account of a managed guest,
//! the server keeps its bundled snapshot for its whole lifetime, so models newer
//! than that snapshot are missing from the run that selected them.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use wait_timeout::ChildExt;

const REFRESH_TIMEOUT: Duration = Duration::from_secs(30);

/// Fill an account's empty models cache with the official CLI before one of its
/// servers starts. Best effort: a failure leaves the snapshot catalog, whose
/// unavailable-model error the run reports as before.
pub(crate) fn warm_opencode_models_cache(environment: &BTreeMap<String, String>) {
    let Some(cache) = models_cache_path(environment) else {
        return;
    };
    if cache.exists() {
        return;
    }
    let outcome = refresh_models_cache(environment);
    crate::logging::info_with_fields(
        "daemon.provider.opencode",
        "refreshed cold OpenCode models cache",
        serde_json::json!({
            "cache_ready": cache.exists(),
            "error": outcome.err(),
        }),
    );
}

fn models_cache_path(environment: &BTreeMap<String, String>) -> Option<PathBuf> {
    environment
        .get("XDG_CACHE_HOME")
        .filter(|value| !value.is_empty())
        .map(|cache| PathBuf::from(cache).join("opencode").join("models.json"))
}

fn refresh_models_cache(environment: &BTreeMap<String, String>) -> Result<(), String> {
    let executable = super::resolve_opencode_executable().map_err(|error| error.to_string())?;
    let mut command = crate::provider::managed_isolated_utility_command(
        executable.display().to_string(),
        vec!["models".to_string(), "--refresh".to_string()],
        environment.clone(),
        None,
        "opencode:models-refresh",
    )
    .map_err(|error| error.to_string())?;
    for name in crate::account_profile::provider_auth_env_vars("opencode") {
        command.env_remove(name);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child =
        crate::process_spawn::spawn_command(command).map_err(|error| error.to_string())?;
    match child.wait_timeout(REFRESH_TIMEOUT) {
        Ok(Some(status)) if status.success() => Ok(()),
        Ok(Some(status)) => Err(format!("models refresh exited with {status}")),
        Ok(None) => {
            let _ = crate::runtime::process_health::terminate_process_tree(child.id());
            let _ = child.wait();
            Err("models refresh timed out".to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}
