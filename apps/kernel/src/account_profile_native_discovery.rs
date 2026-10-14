//! MP-08 / MP-11: host-native provider logins appear without a configuration
//! step. A native scope is the provider's effective default credential scope on
//! the kernel host, the same scope `import-native` registers. Discovery only
//! registers a scope that holds a native login, never copies, logs out or
//! enrolls anything, and keeps a user's removal until they import it again.

use std::time::{Duration, Instant, UNIX_EPOCH};

use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct DismissedNativeScope {
    owner_user_id: String,
    provider: String,
    locator: ProviderAccountLocator,
}

/// In-memory progress of discovery passes for one registry.
#[derive(Default)]
pub(super) struct NativeDiscoveryState {
    in_flight: bool,
    last_pass: Option<Instant>,
    /// Native login fingerprint already observed per (owner, provider).
    observed: BTreeMap<(String, String), String>,
}

impl ProviderAccountProfileRegistry {
    /// Claims the next discovery pass unless one is running or ran recently.
    pub(crate) fn begin_native_discovery_pass(&self, interval: Duration) -> bool {
        let mut state = self.native_discovery_state();
        if state.in_flight
            || state
                .last_pass
                .is_some_and(|last| last.elapsed() < interval)
        {
            return false;
        }
        state.in_flight = true;
        true
    }

    pub(crate) fn finish_native_discovery_pass(&self) {
        let mut state = self.native_discovery_state();
        state.in_flight = false;
        state.last_pass = Some(Instant::now());
    }

    /// Registers the provider's native scope when it holds a login that this
    /// registry has not observed yet, returning the profile to observe and the
    /// login fingerprint to record once it has been observed.
    pub(crate) fn native_scope_needing_observation(
        &self,
        owner_user_id: &str,
        provider: &str,
        home: &Path,
    ) -> Result<Option<(ProviderAccountProfile, String)>, DaemonError> {
        let provider = normalize_provider(provider)?;
        if crate::provider::managed_provider_isolation_required() {
            return Ok(None);
        }
        let locator = ProviderAccountLocator::effective_default(provider, home)?;
        if locator.roots().iter().any(|root| !root.is_absolute()) {
            return Ok(None);
        }
        let key = (owner_user_id.to_string(), provider.to_string());
        let Some(fingerprint) = native_login_fingerprint(&locator) else {
            self.native_discovery_state().observed.remove(&key);
            return Ok(None);
        };
        if self.native_discovery_state().observed.get(&key) == Some(&fingerprint) {
            return Ok(None);
        }
        let dismissed = self
            .read_document()?
            .dismissed_native_scopes
            .iter()
            .any(|scope| {
                scope.owner_user_id == owner_user_id
                    && scope.provider == provider
                    && scope.locator == locator
            });
        if dismissed {
            return Ok(None);
        }
        Ok(Some((
            self.register_native_scope(owner_user_id, provider, locator)?,
            fingerprint,
        )))
    }

    pub(crate) fn record_native_scope_observed(
        &self,
        owner_user_id: &str,
        provider: &str,
        fingerprint: String,
    ) {
        self.native_discovery_state().observed.insert(
            (owner_user_id.to_string(), provider.to_string()),
            fingerprint,
        );
    }

    /// Makes a signed-in profile the provider default when the current default
    /// cannot start work, so the waiting room selects a usable account.
    pub(crate) fn prefer_authenticated_default(
        &self,
        owner_user_id: &str,
        provider: &str,
        profile_id: &str,
    ) -> Result<bool, DaemonError> {
        let provider = normalize_provider(provider)?;
        let document = self.read_document()?;
        let authenticated = |profile: &&StoredProviderAccountProfile| {
            profile.public.auth_state == ProviderAccountAuthState::Authenticated
        };
        let candidate = document.profiles.iter().find(|profile| {
            profile.public.owner_user_id == owner_user_id
                && profile.public.provider == provider
                && profile.public.profile_id == profile_id
        });
        let default_ready = document
            .profiles
            .iter()
            .filter(|profile| {
                profile.public.owner_user_id == owner_user_id
                    && profile.public.provider == provider
                    && profile.public.is_default
            })
            .any(|profile| authenticated(&profile));
        let promote = candidate
            .is_some_and(|profile| authenticated(&profile) && !profile.public.is_default)
            && !default_ready;
        drop(document);
        if promote {
            self.set_default(owner_user_id, provider, profile_id)?;
        }
        Ok(promote)
    }

    fn native_discovery_state(&self) -> std::sync::MutexGuard<'_, NativeDiscoveryState> {
        self.native_discovery
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(super) fn dismiss_native_scope(
    document: &mut RegistryDocument,
    removed: &StoredProviderAccountProfile,
) {
    if removed.public.origin != ProviderAccountProfileOrigin::Default {
        return;
    }
    let scope = DismissedNativeScope {
        owner_user_id: removed.public.owner_user_id.clone(),
        provider: removed.public.provider.clone(),
        locator: removed.locator.clone(),
    };
    if !document.dismissed_native_scopes.contains(&scope) {
        document.dismissed_native_scopes.push(scope);
    }
}

pub(super) fn restore_native_scope(
    document: &mut RegistryDocument,
    owner_user_id: &str,
    provider: &str,
    locator: &ProviderAccountLocator,
) {
    document.dismissed_native_scopes.retain(|scope| {
        !(scope.owner_user_id == owner_user_id
            && scope.provider == provider
            && &scope.locator == locator)
    });
}

/// Non-secret evidence that a native login exists: credential file metadata,
/// or on macOS the Keychain item's attributes. Credential contents are never
/// read here.
fn native_login_fingerprint(locator: &ProviderAccountLocator) -> Option<String> {
    match locator {
        ProviderAccountLocator::Codex { codex_home } => {
            file_fingerprint(&codex_home.join("auth.json"))
        }
        ProviderAccountLocator::Claude {
            claude_config_dir,
            ambient_default,
        } => file_fingerprint(&claude_config_dir.join(".credentials.json")).or_else(|| {
            claude_keychain_fingerprint(claude_config_dir, ambient_default.unwrap_or(false))
        }),
        ProviderAccountLocator::Opencode { xdg_data_home, .. } => {
            file_fingerprint(&xdg_data_home.join("opencode").join("auth.json"))
        }
    }
}

fn file_fingerprint(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() == 0 {
        return None;
    }
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(format!("file:{}:{}", metadata.len(), modified.as_nanos()))
}

#[cfg(target_os = "macos")]
fn claude_keychain_fingerprint(claude_config_dir: &Path, ambient_default: bool) -> Option<String> {
    // Without `-w`/`-g`, `security` prints item attributes only, never the secret.
    claude_keychain_service_names(claude_config_dir, ambient_default)
        .into_iter()
        .find_map(|service| {
            let output = std::process::Command::new("security")
                .args(["find-generic-password", "-s", &service])
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
                .ok()?;
            output.status.success().then(|| {
                let modified = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .find(|line| line.contains("\"mdat\""))
                    .map(str::trim)
                    .unwrap_or_default()
                    .to_string();
                format!("keychain:{service}:{modified}")
            })
        })
}

#[cfg(not(target_os = "macos"))]
fn claude_keychain_fingerprint(
    _claude_config_dir: &Path,
    _ambient_default: bool,
) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        home: PathBuf,
        registry: ProviderAccountProfileRegistry,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn fixture(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "chariox-native-discovery-{name}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let home = root.join("home");
        fs::create_dir_all(&home).expect("fixture home");
        for name in [
            "CODEX_HOME",
            "CLAUDE_CONFIG_DIR",
            "XDG_DATA_HOME",
            "XDG_CONFIG_HOME",
        ] {
            std::env::remove_var(name);
        }
        let registry = ProviderAccountProfileRegistry::open(root.join("profiles.json"))
            .expect("fixture registry");
        Fixture {
            root,
            home,
            registry,
        }
    }

    fn write_codex_login(home: &Path, contents: &str) {
        fs::create_dir_all(home.join(".codex")).expect("codex home");
        // A synthetic non-credential marker; discovery only reads metadata.
        fs::write(home.join(".codex").join("auth.json"), contents).expect("codex login marker");
    }

    #[test]
    fn mp08_native_scope_registers_only_with_a_native_login_and_once_per_login() {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let fixture = fixture("once");
        let managed = fixture
            .registry
            .create_managed("local", "codex", "work")
            .expect("managed profile");

        assert!(fixture
            .registry
            .native_scope_needing_observation("local", "codex", &fixture.home)
            .unwrap()
            .is_none());
        assert_eq!(
            fixture.registry.list("local", Some("codex")).unwrap().len(),
            1
        );

        write_codex_login(&fixture.home, "{}");
        let (native, fingerprint) = fixture
            .registry
            .native_scope_needing_observation("local", "codex", &fixture.home)
            .unwrap()
            .expect("a native login is registered without an import step");
        assert_eq!(native.origin, ProviderAccountProfileOrigin::Default);
        assert!(
            !native.is_default,
            "an existing default is kept until auth is observed"
        );
        assert_ne!(native.profile_id, managed.profile_id);
        fixture
            .registry
            .record_native_scope_observed("local", "codex", fingerprint);
        assert!(fixture
            .registry
            .native_scope_needing_observation("local", "codex", &fixture.home)
            .unwrap()
            .is_none());

        write_codex_login(&fixture.home, "{\"changed\":true}");
        let (again, _) = fixture
            .registry
            .native_scope_needing_observation("local", "codex", &fixture.home)
            .unwrap()
            .expect("a changed native login is observed again");
        assert_eq!(again.profile_id, native.profile_id);
        assert_eq!(
            fixture.registry.list("local", Some("codex")).unwrap().len(),
            2
        );
    }

    #[test]
    fn mp08_removed_native_scope_stays_removed_until_imported_again() {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let fixture = fixture("dismiss");
        write_codex_login(&fixture.home, "{}");
        let (native, _) = fixture
            .registry
            .native_scope_needing_observation("local", "codex", &fixture.home)
            .unwrap()
            .expect("native scope");
        fixture
            .registry
            .remove_registration("local", "codex", &native.profile_id)
            .expect("remove");
        write_codex_login(&fixture.home, "{\"newer\":true}");
        assert!(fixture
            .registry
            .native_scope_needing_observation("local", "codex", &fixture.home)
            .unwrap()
            .is_none());
        let reopened = ProviderAccountProfileRegistry::open(fixture.root.join("profiles.json"))
            .expect("reopen");
        assert!(
            reopened
                .native_scope_needing_observation("local", "codex", &fixture.home)
                .unwrap()
                .is_none(),
            "the removal is durable"
        );

        let imported = reopened
            .import_native_default("local", "codex", &fixture.home)
            .expect("explicit import");
        write_codex_login(&fixture.home, "{\"newest\":true}");
        let (rediscovered, _) = reopened
            .native_scope_needing_observation("local", "codex", &fixture.home)
            .unwrap()
            .expect("an explicit import clears the removal");
        assert_eq!(rediscovered.profile_id, imported.profile_id);
    }

    #[test]
    fn mp08_signed_in_native_account_replaces_an_unusable_default_only() {
        crate::test_support::isolated_env_test!();
        let _guard = crate::env_lock::lock();
        let fixture = fixture("default");
        let managed = fixture
            .registry
            .create_managed("local", "codex", "work")
            .expect("managed profile");
        write_codex_login(&fixture.home, "{}");
        let (native, _) = fixture
            .registry
            .native_scope_needing_observation("local", "codex", &fixture.home)
            .unwrap()
            .expect("native scope");
        assert!(
            !fixture
                .registry
                .prefer_authenticated_default("local", "codex", &native.profile_id)
                .unwrap(),
            "an unobserved native account is not promoted"
        );
        let observe = |profile_id: &str| {
            fixture
                .registry
                .update_observation(
                    "local",
                    "codex",
                    profile_id,
                    ProviderAccountAuthState::Authenticated,
                    None,
                    None,
                    None,
                    None,
                )
                .expect("observe")
        };
        observe(&native.profile_id);
        assert!(fixture
            .registry
            .prefer_authenticated_default("local", "codex", &native.profile_id)
            .unwrap());
        let default = fixture
            .registry
            .get("local", "codex", "default")
            .expect("default");
        assert_eq!(default.profile_id, native.profile_id);

        fixture
            .registry
            .set_default("local", "codex", &managed.profile_id)
            .unwrap();
        observe(&managed.profile_id);
        assert!(
            !fixture
                .registry
                .prefer_authenticated_default("local", "codex", &native.profile_id)
                .unwrap(),
            "a signed-in default chosen by the user is kept"
        );
    }
}
