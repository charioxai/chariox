//! MP-08 / MP-10 / MP-11: execution-local harness inventory, never an importer.
//!
//! Construct the binding from the selected account's resolved launch plan on
//! the execution kernel. No process-global HOME or implementation-agent profile
//! fallback is allowed. Candidates alone never authorize delivery suppression.
use crate::{
    error::DaemonError,
    skill::{frontmatter::parse_frontmatter, package_skill_directory, v2::*},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SkillInventoryBinding {
    pub provider: String,
    pub provider_version: String,
    pub account_profile: String,
    pub execution_id: String,
    pub cwd: PathBuf,
    /// Only skill-discovery launch environment (HOME/config/XDG), not auth vars.
    pub environment: BTreeMap<String, String>,
    pub project_id: Option<String>,
    pub catalog_revision: u64,
    /// Increment on provider source/settings/watch invalidation.
    pub source_revision: u64,
}

#[derive(Debug, Clone)]
pub struct NativeSkillCandidate {
    pub name: String,
    pub description: String,
    pub skill_file: PathBuf,
    pub exposure: NativeSkillExposure,
    pub supports_load_ack: bool,
    /// Hash of body actually retained by the harness (OpenCode inventory).
    pub observed_body_digest: Option<String>,
    pub discovery_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NativeSkillInventory {
    pub entries: Vec<NativeSkillEntry>,
    pub revision: String,
    // Execution-local paths are deliberately absent from the serialized entry.
    paths: BTreeMap<String, PathBuf>,
}

fn failure(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "provider.skill_inventory",
        message: message.into(),
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl SkillInventoryBinding {
    fn validate(&self) -> Result<(), DaemonError> {
        if !self.cwd.is_absolute()
            || self.provider_version.is_empty()
            || self.account_profile.is_empty()
            || self.execution_id.is_empty()
            || !matches!(self.provider.as_str(), "codex" | "claude" | "opencode")
        {
            return Err(failure(
                "native inventory requires a resolved execution/account/version/cwd binding",
            ));
        }
        if self
            .environment
            .keys()
            .any(|k| !execution::DISCOVERY_ENVIRONMENT.contains(&k.as_str()))
        {
            return Err(failure(
                "native inventory environment must contain only skill discovery settings",
            ));
        }
        Ok(())
    }
}

impl NativeSkillInventory {
    pub fn from_candidates(
        binding: &SkillInventoryBinding,
        candidates: Vec<NativeSkillCandidate>,
    ) -> Result<Self, DaemonError> {
        binding.validate()?;
        let mut entries = Vec::new();
        let mut paths = BTreeMap::new();
        for candidate in candidates {
            let entry_id =
                hash(format!("{}\0{}", candidate.name, candidate.skill_file.display()).as_bytes());
            if paths.contains_key(&entry_id) {
                continue;
            }
            let mut exposure = candidate.exposure;
            let mut discovery_error = candidate.discovery_error;
            let mut digest = None;
            let mut default_state = SkillState::Auto;
            let mut complete_manifest = false;
            if candidate.skill_file.is_absolute()
                && candidate
                    .skill_file
                    .file_name()
                    .is_some_and(|n| n == "SKILL.md")
            {
                let packaged = candidate
                    .skill_file
                    .parent()
                    .ok_or_else(|| failure("native skill has no package root"))
                    .and_then(package_skill_directory);
                if let Ok(package) = packaged {
                    use base64::Engine;
                    let body = package
                        .files
                        .iter()
                        .find(|f| f.path == "SKILL.md")
                        .and_then(|f| {
                            base64::engine::general_purpose::STANDARD
                                .decode(&f.content_base64)
                                .ok()
                        })
                        .and_then(|bytes| String::from_utf8(bytes).ok());
                    let metadata = body
                        .as_deref()
                        .and_then(|body| parse_frontmatter(body).ok());
                    if let Some(metadata) = metadata.filter(|m| m.name == candidate.name) {
                        default_state = match binding.provider.as_str() {
                            "codex" => match codex_invocation_state(&package) {
                                Ok(state) => state,
                                Err(_) => {
                                    exposure = NativeSkillExposure::Unverified;
                                    discovery_error =
                                        Some("Codex native invocation policy is unverified".into());
                                    SkillState::Auto
                                }
                            },
                            // Pinned OpenCode 1.19.0 only uses name/description;
                            // it ignores Claude invocation-control frontmatter.
                            "opencode" => SkillState::Auto,
                            _ => metadata.default_state,
                        };
                        let observed_matches =
                            candidate
                                .observed_body_digest
                                .as_ref()
                                .is_none_or(|expected| {
                                    body.as_deref().and_then(native_body).is_some_and(|body| {
                                        &hash(body.trim().as_bytes()) == expected
                                    })
                                });
                        if observed_matches {
                            digest = Some(package.version_hash);
                            complete_manifest = true;
                        } else {
                            discovery_error = Some(
                                "harness retains a different native body; refresh native discovery"
                                    .into(),
                            );
                        }
                    }
                }
            }
            if !complete_manifest {
                if exposure != NativeSkillExposure::Disabled {
                    exposure = NativeSkillExposure::Unverified;
                }
                if discovery_error.is_none() {
                    discovery_error = Some(
                        "complete native package or invocation metadata could not be verified"
                            .into(),
                    );
                }
            }
            paths.insert(entry_id.clone(), candidate.skill_file);
            entries.push(NativeSkillEntry {
                selection: SkillSelection::Native {
                    entry_id,
                    name: candidate.name,
                    provider: binding.provider.clone(),
                    account_profile: binding.account_profile.clone(),
                    execution_id: binding.execution_id.clone(),
                    inventory_revision: String::new(),
                },
                description: candidate.description,
                digest,
                exposure,
                default_state,
                complete_manifest,
                supports_load_ack: candidate.supports_load_ack,
                discovery_error,
            });
        }
        entries.sort_by(|a, b| {
            serde_json::to_string(&a.selection)
                .unwrap()
                .cmp(&serde_json::to_string(&b.selection).unwrap())
        });
        // The fingerprint binds all discovery inputs and package contents/modes.
        let revision = hash(
            format!(
                "{binding:?}\0{}",
                serde_json::to_string(&entries)
                    .map_err(|_| failure("cannot fingerprint native inventory"))?
            )
            .as_bytes(),
        );
        for entry in &mut entries {
            if let SkillSelection::Native {
                inventory_revision, ..
            } = &mut entry.selection
            {
                *inventory_revision = revision.clone();
            }
        }
        Ok(Self {
            entries,
            revision,
            paths,
        })
    }

    /// Resolve only an exact, current, exposure-confirmed selection. The caller
    /// must retain this snapshot's binding to its active account/execution.
    pub fn native_path(&self, selection: &SkillSelection) -> Result<&std::path::Path, DaemonError> {
        let entry = self
            .entries
            .iter()
            .find(|entry| {
                &entry.selection == selection && entry.exposure == NativeSkillExposure::Confirmed
            })
            .ok_or_else(|| {
                failure("native inventory changed or exposure is unverified; refresh and reselect")
            })?;
        let SkillSelection::Native { entry_id, .. } = &entry.selection else {
            return Err(failure("expected native selection"));
        };
        let path = self
            .paths
            .get(entry_id)
            .ok_or_else(|| failure("native execution path is unavailable"))?;
        let package = package_skill_directory(
            path.parent()
                .ok_or_else(|| failure("native skill root is unavailable"))?,
        )?;
        if entry.digest.as_ref() != Some(&package.version_hash) {
            return Err(failure(
                "native package changed since inventory; refresh and reselect",
            ));
        }
        Ok(path.as_path())
    }
}

/// Short-lived metadata cache; rehash complete packages on every read. Explicit
/// refresh, publication, profile/cwd/project/source/version changes all reload
/// the official harness inventory. The query receives Codex's forceReload flag.
#[derive(Default)]
pub struct NativeSkillInventoryCache {
    cached: Option<(
        SkillInventoryBinding,
        Instant,
        Vec<NativeSkillCandidate>,
        String,
    )>,
}
impl NativeSkillInventoryCache {
    pub fn invalidate(&mut self) {
        self.cached = None;
    }
    pub fn inventory(
        &mut self,
        binding: &SkillInventoryBinding,
        refresh: bool,
        query: impl FnOnce(bool) -> Result<Vec<NativeSkillCandidate>, DaemonError>,
    ) -> Result<NativeSkillInventory, DaemonError> {
        binding.validate()?;
        if !refresh {
            if let Some((old, at, candidates, fingerprint)) = &self.cached {
                if old == binding && at.elapsed() < Duration::from_secs(2) {
                    let current =
                        NativeSkillInventory::from_candidates(binding, candidates.clone())?;
                    if &current.revision == fingerprint {
                        return Ok(current);
                    }
                    // A changed complete package is an invalidation even if
                    // the provider's file watcher or a catalog event was lost.
                }
            }
        }
        let previous = self.cached.take();
        let candidates = match query(true) {
            Ok(candidates) => candidates,
            Err(error) => {
                if let Some((old, _, mut candidates, _)) =
                    previous.filter(|(old, _, entries, _)| old == binding && !entries.is_empty())
                {
                    for candidate in &mut candidates {
                        if candidate.exposure != NativeSkillExposure::Disabled {
                            candidate.exposure = NativeSkillExposure::Unverified;
                        }
                        candidate.discovery_error = Some("official native inventory unavailable; exposure remains unverified until refresh succeeds".into());
                    }
                    // Do not cache failure or retain stale selection authority.
                    return NativeSkillInventory::from_candidates(&old, candidates);
                }
                return Err(error);
            }
        };
        let inventory = NativeSkillInventory::from_candidates(binding, candidates.clone())?;
        self.cached = Some((
            binding.clone(),
            Instant::now(),
            candidates,
            inventory.revision.clone(),
        ));
        Ok(inventory)
    }
}

/// Official Codex skills/list response: only actual-cwd results count. An error
/// in that cwd prevents claiming exposure even if a partial list is present.
pub fn codex_skill_candidates(
    response: &Value,
    cwd: &std::path::Path,
) -> Result<Vec<NativeSkillCandidate>, DaemonError> {
    let data = response
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| failure("Codex skills/list has no cwd inventory"))?;
    let item = data
        .iter()
        .find(|v| {
            v.get("cwd")
                .and_then(Value::as_str)
                .map(std::path::Path::new)
                == Some(cwd)
        })
        .ok_or_else(|| failure("Codex skills/list omitted execution cwd"))?;
    let errors = item
        .get("errors")
        .and_then(Value::as_array)
        .ok_or_else(|| failure("Codex skills/list omitted discovery errors"))?;
    let skills = item
        .get("skills")
        .and_then(Value::as_array)
        .ok_or_else(|| failure("Codex skills/list omitted skills"))?;
    Ok(skills
        .iter()
        .filter_map(|v| {
            Some(NativeSkillCandidate {
                name: v.get("name")?.as_str()?.into(),
                description: v.get("description")?.as_str()?.into(),
                skill_file: PathBuf::from(v.get("path").and_then(Value::as_str).unwrap_or("")),
                exposure: match v.get("enabled").and_then(Value::as_bool) {
                    Some(false) => NativeSkillExposure::Disabled,
                    Some(true) if errors.is_empty() => NativeSkillExposure::Confirmed,
                    _ => NativeSkillExposure::Unverified,
                },
                // A listing is availability, not an observed native skill load.
                supports_load_ack: true,
                observed_body_digest: None,
                discovery_error: (!errors.is_empty())
                    .then(|| "Codex reported skill discovery errors".into()),
            })
        })
        .collect())
}

mod execution;
pub(crate) use execution::capture_launch_skill_environment;
pub use execution::{inventory_for_run, NativeSkillDiscoveryRequest};
mod roots;
pub use roots::filesystem_skill_candidates;

#[cfg(test)]
mod tests;

// Body retained by OpenCode's ConfigMarkdown parser, without YAML frontmatter.
fn native_body(body: &str) -> Option<&str> {
    let body = body
        .strip_prefix("---\n")
        .or_else(|| body.strip_prefix("---\r\n"))?;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        offset += line.len();
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return Some(&body[offset..]);
        }
    }
    None
}
fn codex_invocation_state(
    package: &crate::skill::CharioxSkillPackage,
) -> Result<SkillState, DaemonError> {
    use base64::Engine;
    let Some(policy) = package
        .files
        .iter()
        .find(|f| f.path == "agents/openai.yaml")
    else {
        return Ok(SkillState::Auto);
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&policy.content_base64)
        .map_err(|_| failure("cannot inspect Codex skill invocation policy"))?;
    let yaml: serde_yaml::Value = serde_yaml::from_slice(&bytes)
        .map_err(|_| failure("cannot verify Codex skill invocation policy"))?;
    match yaml
        .get("policy")
        .and_then(|v| v.get("allow_implicit_invocation"))
    {
        None | Some(serde_yaml::Value::Bool(true)) => Ok(SkillState::Auto),
        Some(serde_yaml::Value::Bool(false)) => Ok(SkillState::Manual),
        _ => Err(failure(
            "cannot verify Codex implicit-invocation visibility",
        )),
    }
}
