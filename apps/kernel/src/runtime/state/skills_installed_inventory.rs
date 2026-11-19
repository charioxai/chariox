//! The skills of every installed provider, listed for a Project with no run.
//! A provider is installed when its executable resolves, as provider setup checks.
//! Each listing uses the provider's default account and its own documented tool:
//! Claude's skill folders, Codex skills/list and OpenCode's /skill on the managed
//! account server the model catalog already uses, OpenCode's folders otherwise.
use super::*;
use crate::provider::skill_inventory::{
    capture_launch_skill_environment, inventory_for_binding, NativeSkillInventory,
    NativeSkillInventoryCache, ProviderListing, SkillInventoryBinding,
};
use crate::skill::v2::{NativeSkillEntry, SkillSelection};
use std::sync::Mutex as StdMutex;

const PROVIDERS: [&str; 3] = ["claude", "codex", "opencode"];
const EXECUTION_PREFIX: &str = "installed:";

#[derive(Default)]
struct InstalledInventory {
    version: Option<String>,
    inventory: NativeSkillInventoryCache,
}

#[derive(Default)]
pub(super) struct InstalledSkillInventories {
    listings: StdMutex<BTreeMap<String, Arc<StdMutex<InstalledInventory>>>>,
}

impl InstalledSkillInventories {
    fn listing(&self, execution_id: &str) -> Arc<StdMutex<InstalledInventory>> {
        let mut listings = self
            .listings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !listings.contains_key(execution_id) && listings.len() >= 128 {
            if let Some(key) = listings.keys().next().cloned() {
                listings.remove(&key);
            }
        }
        listings.entry(execution_id.into()).or_default().clone()
    }
}

fn failure(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "skills.v2.installed_inventory",
        message: message.into(),
    }
}

/// `installed:<provider>:<kernel>:<Project>`: what Details and Import need to list
/// the same skills again without a run.
pub(super) fn installed_execution_id(provider: &str, daemon_id: &str, project_id: &str) -> String {
    format!("{EXECUTION_PREFIX}{provider}:{daemon_id}:{project_id}")
}

/// The provider and Project an installed execution id names on this kernel.
pub(super) struct InstalledExecution {
    pub provider: &'static str,
    pub project_id: String,
}

pub(super) fn parse_installed_execution_id(
    execution_id: &str,
    daemon_id: &str,
) -> Option<Result<InstalledExecution, DaemonError>> {
    let rest = execution_id.strip_prefix(EXECUTION_PREFIX)?;
    let parsed = rest.split_once(':').and_then(|(provider, rest)| {
        let provider = PROVIDERS.into_iter().find(|known| *known == provider)?;
        let project_id = rest.strip_prefix(daemon_id)?.strip_prefix(':')?;
        (!project_id.is_empty()).then(|| InstalledExecution {
            provider,
            project_id: project_id.into(),
        })
    });
    Some(parsed.ok_or_else(|| {
        failure("this provider skill was listed by another kernel or Project; refresh Skills")
    }))
}

fn installed_executable(provider: &str) -> Result<PathBuf, DaemonError> {
    match provider {
        "claude" => crate::provider::resolve_claude_executable(),
        "codex" => crate::provider::resolve_codex_executable(),
        "opencode" => crate::provider::resolve_opencode_executable(),
        _ => Err(failure("this provider has no native skills inventory")),
    }
}

impl KernelRuntimeState {
    /// The skills of every installed provider for the Project, except the families in
    /// `listed_by_run`, whose focused run already lists them. A provider that is not
    /// installed lists nothing; one whose listing fails is logged and left out.
    pub(super) fn installed_native_entries(
        &self,
        user: &str,
        project_id: &str,
        catalog_revision: u64,
        listed_by_run: Option<&str>,
    ) -> Vec<NativeSkillEntry> {
        let mut entries = Vec::new();
        for provider in PROVIDERS {
            if listed_by_run == Some(provider) || installed_executable(provider).is_err() {
                continue;
            }
            match self.installed_skill_inventory(user, provider, project_id, catalog_revision) {
                Ok(inventory) => entries.extend(inventory.entries),
                Err(error) => crate::logging::warn_with_fields(
                    "daemon.skills",
                    "installed provider skills could not be listed",
                    serde_json::json!({
                        "provider": provider,
                        "project_id": project_id,
                        "error": error.to_string(),
                    }),
                ),
            }
        }
        entries
    }

    /// The inventory a Details or Import selection of an installed provider names.
    pub(super) fn installed_skill_inventory_for(
        &self,
        user: &str,
        selection: &SkillSelection,
        catalog_revision: u64,
    ) -> Option<Result<NativeSkillInventory, DaemonError>> {
        let SkillSelection::Native { execution_id, .. } = selection else {
            return None;
        };
        let daemon_id = self.owned.config_projection.snapshot().daemon_id;
        let execution = parse_installed_execution_id(execution_id, &daemon_id)?;
        Some(execution.and_then(|execution| {
            self.installed_skill_inventory(
                user,
                execution.provider,
                &execution.project_id,
                catalog_revision,
            )
        }))
    }

    fn installed_skill_inventory(
        &self,
        user: &str,
        provider: &'static str,
        project_id: &str,
        catalog_revision: u64,
    ) -> Result<NativeSkillInventory, DaemonError> {
        let executable = installed_executable(provider)?;
        self.ensure_skill_project_visible(Some(project_id), user)?;
        let project = self.owned.session_store.get_project(project_id)?;
        let folder = project
            .workspace_ids()
            .first()
            .map(String::as_str)
            .unwrap_or_else(|| project.workspace_id());
        // A Project with no folder lists only the provider's account skills: they are
        // listed from an empty folder of its own.
        let folder = if folder.is_empty() {
            let empty = std::env::temp_dir().join("chariox-skills-no-project-folder");
            std::fs::create_dir_all(&empty)
                .map_err(|_| failure("the folder for account skills could not be created"))?;
            empty
        } else {
            PathBuf::from(folder)
        };
        let root = std::fs::canonicalize(folder)
            .map_err(|_| failure("the Project folder is unavailable"))?;
        let owner = self.provider_account_authority_owner_user_id(user);
        let registry = &self.owned.provider_account_profiles;
        let default_profile = registry
            .list(&owner, Some(provider))?
            .into_iter()
            .find(|profile| profile.is_default);
        // With no account set up, the provider runs on its own default home.
        let (account_profile, account_environment) = match default_profile {
            Some(profile) => {
                let environment =
                    registry.resolve_environment(&owner, provider, &profile.profile_id)?;
                (profile.profile_id, environment)
            }
            None => ("default".to_string(), BTreeMap::new()),
        };
        let environment = capture_launch_skill_environment(&account_environment, &[]);
        let daemon_id = self.owned.config_projection.snapshot().daemon_id;
        let execution_id = installed_execution_id(provider, &daemon_id, project_id);
        let endpoint = match provider {
            "codex" => Some(crate::provider::ensure_codex_account_endpoint(
                &owner,
                &account_profile,
                account_environment,
            )?),
            "opencode" => Some(crate::provider::ensure_opencode_account_endpoint(
                &owner,
                &account_profile,
                account_environment,
            )?),
            _ => None,
        };
        let listing = self
            .owned
            .installed_skill_inventories
            .listing(&execution_id);
        let mut listing = listing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if listing.version.is_none() {
            listing.version = Some(match (provider, endpoint.as_deref()) {
                ("codex", Some(endpoint)) => {
                    crate::provider::CodexClient::new(&execution_id, endpoint)?
                        .native_skill_server_version()?
                }
                ("opencode", Some(endpoint)) => {
                    crate::provider::OpenCodeClient::new(&execution_id, endpoint)?
                        .native_skill_server_version()?
                }
                _ => crate::provider::probe_claude_skill_version(&executable, &environment)?,
            });
        }
        let binding = SkillInventoryBinding {
            provider: provider.into(),
            provider_version: listing.version.clone().expect("resolved above"),
            account_profile,
            execution_id: execution_id.clone(),
            cwd: root.clone(),
            environment,
            project_id: Some(project_id.into()),
            catalog_revision,
            source_revision: 0,
            // No process yet: a provider started now loads every skill on disk.
            started_at_ms: u64::MAX,
        };
        inventory_for_binding(
            &binding,
            &mut listing.inventory,
            ProviderListing {
                client_id: &execution_id,
                endpoint: endpoint.as_deref(),
                opencode_agent: "build",
            },
            &root,
            &[],
            false,
        )
    }
}

#[cfg(test)]
#[path = "skills_installed_inventory_tests.rs"]
mod tests;
