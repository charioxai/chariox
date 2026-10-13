//! MP-08 / MP-11: the official provider harness CLIs this kernel host can run.
//!
//! The kernel is the only authority for which providers a client may offer:
//! a provider family whose CLI is not installed is absent from the waiting-room
//! inventory, the provider catalog sources and the relay advertisement.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::claude::claude_harness_installed;
use super::codex::codex_harness_installed;
use super::opencode::opencode_harness_installed;

/// Provider families, in display order, whose official CLI resolves now.
pub(crate) fn installed_provider_harnesses() -> Vec<String> {
    [
        ("claude", claude_harness_installed as fn() -> bool),
        ("codex", codex_harness_installed),
        ("opencode", opencode_harness_installed),
    ]
    .into_iter()
    .filter(|(_, installed)| installed())
    .map(|(family, _)| family.to_string())
    .collect()
}

/// The waiting-room hot path reuses one PATH scan for a few seconds; a CLI
/// installed or removed later appears in the next inventory after that.
const INSTALLED_HARNESS_CACHE_TTL: Duration = Duration::from_secs(5);

pub(crate) fn cached_installed_provider_harnesses() -> Vec<String> {
    static CACHE: OnceLock<Mutex<Option<(Instant, Vec<String>)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut cached = cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((observed_at, installed)) = cached.as_ref() {
        if observed_at.elapsed() < INSTALLED_HARNESS_CACHE_TTL {
            return installed.clone();
        }
    }
    let installed = installed_provider_harnesses();
    *cached = Some((Instant::now(), installed.clone()));
    installed
}

/// Keeps adapter ids whose family harness is installed. Ids outside the three
/// official families (for example the dev stub) follow their own policy.
pub(crate) fn retain_installed_harness_providers(providers: &mut Vec<String>, installed: &[String]) {
    providers.retain(|provider| match super::canonical_provider_family(provider) {
        Some(family @ ("claude" | "codex" | "opencode")) => {
            installed.iter().any(|candidate| candidate == family)
        }
        _ => true,
    });
}

#[cfg(test)]
mod tests {
    use super::{installed_provider_harnesses, retain_installed_harness_providers};

    #[test]
    fn mp08_installed_harnesses_follow_resolvable_cli_overrides() {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-harness-inventory-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("fixture root");
        let codex = root.join("codex");
        std::fs::write(&codex, "#!/bin/sh\nexit 0\n").expect("codex fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755))
                .expect("executable fixture");
        }
        std::env::set_var("CHARIOX_CLAUDE_BIN", root.join("missing-claude"));
        std::env::set_var("CHARIOX_CODEX_BIN", &codex);
        std::env::set_var("CHARIOX_OPENCODE_BIN", root.join("missing-opencode"));

        let installed = installed_provider_harnesses();

        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(installed, vec!["codex".to_string()]);
    }

    #[test]
    fn mp08_advertised_providers_drop_uninstalled_harness_families() {
        let mut providers = [
            "dev-stub", "claude", "claude-headless", "claude-p", "codex", "opencode",
        ]
        .map(str::to_string)
        .to_vec();
        retain_installed_harness_providers(&mut providers, &["codex".to_string()]);
        assert_eq!(providers, vec!["dev-stub".to_string(), "codex".to_string()]);
    }
}
