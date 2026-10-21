//! MP-08 / MP-10 / MP-11: verified instruction caches, never worker catalogs/grants.
use super::{
    catalog::{failure, validate_id},
    package::{materialize_skill_package_for_id, package_skill_directory, verify_package},
    paths::{existing_owned_directory, owned_directory},
    v2::*,
    CharioxSkillPackage,
};
use crate::{
    error::DaemonError,
    transport::relay_peer::{
        RelayPeerResponse, RemoteSkillMaterialization, RemoteSkillSyncContext,
    },
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

fn manifest_valid(manifest: &SkillTransferManifest) -> Result<(), DaemonError> {
    if !manifest.skills.is_empty() && manifest.catalog_revision == 0 {
        return Err(failure(
            "skill transfer requires a published v2 revision; upgrade the home kernel",
        ));
    }
    if let Some(project) = &manifest.project_id {
        validate_id(project)?;
    }
    let mut ids = BTreeSet::new();
    for entry in &manifest.skills {
        validate_id(&entry.skill_id)?;
        if !ids.insert(&entry.skill_id) {
            return Err(failure("duplicate skill ID in transfer manifest"));
        }
        if entry.digest.len() != 64
            || !entry
                .digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(failure("invalid skill transfer digest"));
        }
        if entry.state == SkillState::Off {
            return Err(failure("Off skills cannot be newly transferred"));
        }
        if let SkillScope::Project { project_id } = &entry.scope {
            if manifest.project_id.as_ref() != Some(project_id) {
                return Err(failure("skill transfer contains a foreign Project"));
            }
        }
    }
    Ok(())
}
fn binding_relative(
    context: &RemoteSkillSyncContext,
    lease_id: &str,
) -> Result<PathBuf, DaemonError> {
    for id in [
        &context.home_kernel_id,
        &context.home_session_id,
        &context.home_agent_id,
        &context.leased_agent_id,
    ] {
        validate_id(id)?;
    }
    validate_id(lease_id)?;
    let encoded = serde_json::to_vec(&(context, lease_id))
        .map_err(|_| failure("invalid skill execution binding"))?;
    let execution = format!("execution-{:x}", Sha256::digest(encoded));
    Ok(Path::new("state/skill-materializations")
        .join(&context.home_kernel_id)
        .join(execution))
}
/// The directory of this authenticated execution's verified skill cache. A managed
/// provider is sandboxed with this one root, created before launch because later
/// transfers add packages inside an already-running sandbox.
pub fn execution_cache_root(
    home: &Path,
    context: &RemoteSkillSyncContext,
    lease_id: &str,
) -> Result<PathBuf, DaemonError> {
    owned_directory(home, &binding_relative(context, lease_id)?)
}
/// Renamed copies of one source share a complete-content digest: the receiver
/// materializes one payload per digest for every ID-bound manifest entry.
pub fn unique_payloads(packages: Vec<CharioxSkillPackage>) -> Vec<CharioxSkillPackage> {
    let mut seen = BTreeSet::new();
    packages
        .into_iter()
        .filter(|package| seen.insert(package.version_hash.clone()))
        .collect()
}
/// Caller must first consume the existing authenticated lease admission and
/// compare the supplied execution lease with the current LeasedAgent record.
/// All payloads are checked before the first file is written. Missing bytes may
/// use only this exact binding's verified ID+digest cache, never a same-name skill.
pub fn materialize_received_packages(
    home: &Path,
    context: &RemoteSkillSyncContext,
    lease_id: &str,
    manifest: &SkillTransferManifest,
    packages: &[CharioxSkillPackage],
) -> Result<Vec<RemoteSkillMaterialization>, DaemonError> {
    manifest_valid(manifest)?;
    let relative = binding_relative(context, lease_id)?;
    let expected: BTreeSet<_> = manifest.skills.iter().map(|e| e.digest.as_str()).collect();
    let mut payloads = BTreeMap::new();
    for package in packages {
        verify_package(package)?;
        if !expected.contains(package.version_hash.as_str())
            || payloads
                .insert(package.version_hash.as_str(), package)
                .is_some()
        {
            return Err(failure(
                "skill transfer payload is duplicate or outside the reviewed manifest",
            ));
        }
    }
    if manifest.skills.is_empty() {
        return Ok(vec![]);
    }
    let base = owned_directory(home, &relative)?;
    let mut materialized = Vec::new();
    for entry in &manifest.skills {
        let root = if let Some(package) = payloads.get(entry.digest.as_str()) {
            materialize_skill_package_for_id(&base, &entry.skill_id, package)?
        } else {
            existing_owned_directory(home, &relative.join(&entry.skill_id).join(&entry.digest))
                .map_err(|_| failure("skill execution cache is missing or invalid; resend the exact package before submitting"))?
        };
        let verified=package_skill_directory(&root).map_err(|_|failure("skill execution cache is missing or invalid; resend the exact package before submitting"))?;
        if verified.version_hash != entry.digest {
            return Err(failure(
                "skill execution cache content mismatch; submission refused",
            ));
        }
        materialized.push(RemoteSkillMaterialization {
            skill_id: entry.skill_id.clone(),
            name: verified.metadata.name,
            version_hash: entry.digest.clone(),
            materialized_root: root.to_string_lossy().into(),
        });
    }
    Ok(materialized)
}
/// Receipts prove materialization only, never provider delivery or a Loaded state.
/// Keep the same check at every sender; no caller may accept a merely matching name.
pub fn verify_transfer_receipt(
    expected_context: &RemoteSkillSyncContext,
    expected_lease: &str,
    expected: &SkillTransferManifest,
    response: RelayPeerResponse,
) -> Result<Vec<RemoteSkillMaterialization>, DaemonError> {
    manifest_valid(expected)?;
    binding_relative(expected_context, expected_lease)?;
    let RelayPeerResponse::RemoteSkillPackagesEnsured {
        context,
        execution_lease_id,
        manifest,
        materialized,
    } = response
    else {
        return Err(failure(
            "unexpected skill transfer receipt; submission refused",
        ));
    };
    if &context != expected_context || execution_lease_id != expected_lease || &manifest != expected
    {
        return Err(failure(
            "stale or foreign skill transfer receipt; submission refused",
        ));
    }
    let mut seen = BTreeSet::new();
    if materialized.len() != expected.skills.len() {
        return Err(failure("incomplete skill transfer receipt"));
    }
    for entry in &materialized {
        let expected = expected
            .skills
            .iter()
            .find(|e| e.skill_id == entry.skill_id)
            .ok_or_else(|| failure("unrequested skill in transfer receipt"))?;
        if !seen.insert(&entry.skill_id) || entry.version_hash != expected.digest {
            return Err(failure("skill transfer receipt ID/digest mismatch"));
        }
        let root = Path::new(&entry.materialized_root);
        if !root.is_absolute()
            || root
                .components()
                .any(|p| matches!(p, std::path::Component::ParentDir))
            || entry.materialized_root.chars().any(char::is_control)
        {
            return Err(failure("invalid worker skill execution root"));
        }
    }
    Ok(materialized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    struct Fixture {
        root: PathBuf,
        home: PathBuf,
        context: RemoteSkillSyncContext,
        manifest: SkillTransferManifest,
        package: CharioxSkillPackage,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("skillsk-transfer-{:032x}", rand::random::<u128>()));
            let home = root.join("home");
            let source = root.join("source");
            fs::create_dir_all(&home).unwrap();
            fs::create_dir(&source).unwrap();
            fs::write(source.join("SKILL.md"),"---\nname: review\ndescription: Review current code\n---\nRead helper.sh, but never run it merely on install.\n").unwrap();
            fs::write(source.join("helper.sh"), "#!/bin/sh\necho reviewed\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(source.join("helper.sh"), fs::Permissions::from_mode(0o700))
                    .unwrap();
            }
            let package = package_skill_directory(&source).unwrap();
            let context = RemoteSkillSyncContext {
                home_kernel_id: "home".into(),
                home_session_id: "s1".into(),
                home_agent_id: "a1".into(),
                leased_agent_id: "la1".into(),
            };
            let manifest = SkillTransferManifest {
                catalog_revision: 1,
                project_id: Some("p1".into()),
                skills: vec![SkillTransferEntry {
                    skill_id: "skill-1".into(),
                    digest: package.version_hash.clone(),
                    scope: SkillScope::Project {
                        project_id: "p1".into(),
                    },
                    state: SkillState::Manual,
                }],
            };
            Self {
                root,
                home,
                context,
                manifest,
                package,
            }
        }
        fn send(
            &self,
            context: &RemoteSkillSyncContext,
            lease: &str,
            manifest: &SkillTransferManifest,
            packages: &[CharioxSkillPackage],
        ) -> Result<Vec<RemoteSkillMaterialization>, DaemonError> {
            materialize_received_packages(&self.home, context, lease, manifest, packages)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn skills_v2_transfer_validates_all_members_before_creating_an_execution_cache() {
        let f = Fixture::new();
        let mut bad = f.package.clone();
        bad.files[0].sha256 = "0".repeat(64);
        assert!(f.send(&f.context, "lease1", &f.manifest, &[bad]).is_err());
        assert!(!f.home.join("state").exists());
        assert!(f
            .send(
                &f.context,
                "lease1",
                &SkillTransferManifest::default(),
                &[f.package.clone()]
            )
            .is_err());
        assert!(!f.home.join("state").exists());
    }
    #[test]
    fn skills_v2_transfer_is_private_to_the_whole_execution_binding_without_a_catalog_or_grant() {
        let f = Fixture::new();
        let first = f
            .send(&f.context, "lease1", &f.manifest, &[f.package.clone()])
            .unwrap();
        assert_eq!(first[0].skill_id, "skill-1");
        assert_eq!(first[0].version_hash, f.package.version_hash);
        assert!(Path::new(&first[0].materialized_root)
            .join("helper.sh")
            .is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                fs::metadata(Path::new(&first[0].materialized_root).join("helper.sh"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o111,
                0
            );
        }
        assert_eq!(
            f.send(&f.context, "lease1", &f.manifest, &[]).unwrap(),
            first
        );
        let mut other = f.context.clone();
        other.home_agent_id = "a2".into();
        assert!(f.send(&other, "lease1", &f.manifest, &[]).is_err());
        assert!(f
            .send(&f.context, "replacement-lease", &f.manifest, &[])
            .is_err());
        assert!(!f.home.join("skills").exists());
        assert!(!f.home.join("state/skills").exists());
        assert!(!f.home.join("user/skills").exists());
    }
    #[test]
    fn skills_v2_transfer_sends_one_payload_for_renamed_copies_sharing_a_digest() {
        let f = Fixture::new();
        let mut manifest = f.manifest.clone();
        let mut renamed = manifest.skills[0].clone();
        renamed.skill_id = "skill-2".into();
        manifest.skills.push(renamed);
        // The sender holds one package per skill ID; both carry the same digest.
        let payloads = unique_payloads(vec![f.package.clone(), f.package.clone()]);
        assert_eq!(payloads.len(), 1);
        let roots = f.send(&f.context, "lease1", &manifest, &payloads).unwrap();
        assert_eq!(
            roots
                .iter()
                .map(|r| r.skill_id.as_str())
                .collect::<Vec<_>>(),
            ["skill-1", "skill-2"]
        );
        // The receiver still refuses a repeated payload.
        assert!(f
            .send(
                &f.context,
                "lease2",
                &manifest,
                &[f.package.clone(), f.package.clone()]
            )
            .is_err());
    }
    #[test]
    fn skills_v2_transfer_rejects_off_foreign_project_and_duplicate_ids() {
        let f = Fixture::new();
        let mut manifest = f.manifest.clone();
        manifest.skills[0].state = SkillState::Off;
        assert!(f
            .send(&f.context, "lease1", &manifest, &[f.package.clone()])
            .is_err());
        manifest = f.manifest.clone();
        manifest.project_id = Some("p2".into());
        assert!(f
            .send(&f.context, "lease1", &manifest, &[f.package.clone()])
            .is_err());
        manifest = f.manifest.clone();
        manifest.skills.push(manifest.skills[0].clone());
        assert!(f
            .send(&f.context, "lease1", &manifest, &[f.package.clone()])
            .is_err());
        assert!(!f.home.join("state").exists());
    }
    #[test]
    fn skills_v2_transfer_never_uses_another_binding_or_modified_same_name_copy() {
        let f = Fixture::new();
        let roots = f
            .send(&f.context, "lease1", &f.manifest, &[f.package.clone()])
            .unwrap();
        fs::write(
            Path::new(&roots[0].materialized_root).join("helper.sh"),
            "MODIFIED",
        )
        .unwrap();
        assert!(f.send(&f.context, "lease1", &f.manifest, &[]).is_err());
        assert!(f.send(&f.context, "lease2", &f.manifest, &[]).is_err());
    }
    #[test]
    fn skills_v2_transfer_receipts_bind_lease_project_revision_id_and_digest() {
        let f = Fixture::new();
        let roots = f
            .send(&f.context, "lease1", &f.manifest, &[f.package.clone()])
            .unwrap();
        let receipt =
            |manifest: SkillTransferManifest,
             materialized: Vec<RemoteSkillMaterialization>,
             lease: &str| RelayPeerResponse::RemoteSkillPackagesEnsured {
                context: f.context.clone(),
                execution_lease_id: lease.into(),
                manifest,
                materialized,
            };
        verify_transfer_receipt(
            &f.context,
            "lease1",
            &f.manifest,
            receipt(f.manifest.clone(), roots.clone(), "lease1"),
        )
        .unwrap();
        assert!(verify_transfer_receipt(
            &f.context,
            "lease1",
            &f.manifest,
            receipt(f.manifest.clone(), roots.clone(), "lease2")
        )
        .is_err());
        let mut old = f.manifest.clone();
        old.catalog_revision = 0;
        assert!(verify_transfer_receipt(
            &f.context,
            "lease1",
            &f.manifest,
            receipt(old, roots.clone(), "lease1")
        )
        .is_err());
        let mut wrong = roots;
        wrong[0].skill_id = "foreign".into();
        assert!(verify_transfer_receipt(
            &f.context,
            "lease1",
            &f.manifest,
            receipt(f.manifest.clone(), wrong, "lease1")
        )
        .is_err());
    }
}
