//! MP-08 / MP-10 / MP-11: provider-side, execution-local native Details data.
//! These are internal service values, not an allocation of a new wire shape.
use super::*;
use crate::skill::v2::SkillFileManifest;
use base64::Engine;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeSkillProvenance {
    pub provider: String,
    pub provider_version: String,
    pub account_profile: String,
    pub execution_id: String,
    pub cwd: PathBuf,
}
impl From<&SkillInventoryBinding> for NativeSkillProvenance {
    fn from(binding: &SkillInventoryBinding) -> Self {
        Self {
            provider: binding.provider.clone(),
            provider_version: binding.provider_version.clone(),
            account_profile: binding.account_profile.clone(),
            execution_id: binding.execution_id.clone(),
            cwd: binding.cwd.clone(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeSkillDetails {
    pub entry: NativeSkillEntry,
    pub provenance: NativeSkillProvenance,
    pub root: Option<PathBuf>,
    /// Full source SKILL.md; never a Chariox copy or a provider credential file.
    pub body: Option<String>,
    pub files: Vec<SkillFileManifest>,
    /// Physical package digest may be known even when harness exposure/body
    /// coherence is unverified. It is never selection or load authority.
    pub manifest_digest: Option<String>,
}
impl NativeSkillInventory {
    pub fn details(&self, selection: &SkillSelection) -> Result<NativeSkillDetails, DaemonError> {
        let entry = self
            .entries
            .iter()
            .find(|entry| &entry.selection == selection)
            .ok_or_else(|| failure("native inventory changed; refresh and reselect Details"))?;
        let SkillSelection::Native { entry_id, .. } = selection else {
            return Err(failure("expected native Details selection"));
        };
        let path = self
            .paths
            .get(entry_id)
            .ok_or_else(|| failure("native source path is unavailable"))?;
        let mut details = NativeSkillDetails {
            entry: entry.clone(),
            provenance: self.provenance.clone(),
            root: path
                .is_absolute()
                .then(|| path.parent().map(PathBuf::from))
                .flatten(),
            body: None,
            files: vec![],
            manifest_digest: None,
        };
        let Some(expected) = self.physical_digests.get(entry_id) else {
            return Ok(details);
        };
        let root = details
            .root
            .as_ref()
            .ok_or_else(|| failure("native source package root is unavailable"))?;
        let package = package_skill_directory(root)?;
        if &package.version_hash != expected {
            return Err(failure(
                "native package changed since inventory; refresh Details",
            ));
        }
        for file in package.files {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&file.content_base64)
                .map_err(|_| failure("cannot inspect verified native package file"))?;
            let length = bytes.len() as u64;
            if file.path == "SKILL.md" {
                details.body = Some(
                    String::from_utf8(bytes)
                        .map_err(|_| failure("native SKILL.md is not UTF-8"))?,
                );
            }
            details.files.push(SkillFileManifest {
                path: file.path,
                sha256: file.sha256,
                bytes: length,
                executable: file.executable,
            });
        }
        details.manifest_digest = Some(package.version_hash);
        Ok(details)
    }
}
