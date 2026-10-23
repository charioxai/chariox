//! MP-08 / MP-10 / MP-11: production-service containment regressions; not SV acceptance.
use chariox_kernel::{
    durable_state::DurableKernelStateStore,
    skill::{service::SkillCatalogService, storage::SkillCatalogStorage, v2::*},
};
use std::{fs, path::PathBuf};

const BODY: &str =
    "---\nname: review\ndescription: Review code\n---\nRead the public documentation.\n";
struct Fixture {
    root: PathBuf,
    home: PathBuf,
    service: SkillCatalogService,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "skills-v2-containment-{:032x}",
            rand::random::<u128>()
        ));
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let durable = DurableKernelStateStore::open(home.join("kernel.db")).unwrap();
        let service = SkillCatalogService::new(SkillCatalogStorage::new(&home, durable).unwrap());
        Self {
            root,
            home,
            service,
        }
    }
    fn mutation(&self, source: SkillInstallSource) -> SkillCatalogMutation {
        SkillCatalogMutation {
            scope: SkillScope::Chariox,
            expected_revision: 0,
            actions: vec![SkillCatalogAction::Install {
                source,
                collision: SkillCollisionAction::Reject,
                package_id: None,
            }],
        }
    }
    fn content(&self) -> SkillCatalogMutation {
        self.mutation(SkillInstallSource::Content {
            skill_md: BODY.into(),
        })
    }
    fn install(&self) -> SkillRecord {
        let mutation = self.content();
        let prepared = self.service.prepare(&mutation).unwrap();
        self.service
            .publish_authorized(&mutation, &prepared)
            .unwrap()
            .catalogs[0]
            .skills[0]
            .clone()
    }
    fn turn(&self) -> Result<SkillTurnContext, chariox_kernel::error::DaemonError> {
        self.service.pinned_turn(
            None,
            "agent",
            "session",
            "$review",
            &[],
            &[],
            Some(128_000),
            1.0,
            8192,
            "g1",
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
#[cfg(unix)]
fn skills_v2_containment_rejects_staging_parent_symlink() {
    let f = Fixture::new();
    let outside = f.root.join("workspace");
    fs::create_dir(&outside).unwrap();
    fs::create_dir_all(f.home.join("state")).unwrap();
    std::os::unix::fs::symlink(&outside, f.home.join("state/skill-staging")).unwrap();
    assert!(
        f.service.prepare(&f.content()).is_err(),
        "source preparation must refuse a staging parent outside Chariox state"
    );
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    assert_eq!(f.service.storage.snapshot().unwrap().revision, 0);
}
#[test]
#[cfg(unix)]
fn skills_v2_containment_rejects_install_parent_symlink_without_publication() {
    let f = Fixture::new();
    let mutation = f.content();
    let prepared = f.service.prepare(&mutation).unwrap();
    let outside = f.root.join("workspace");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, f.home.join("skills")).unwrap();
    assert!(
        f.service.publish_authorized(&mutation, &prepared).is_err(),
        "publishing may not create packages through a parent symlink"
    );
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    assert_eq!(f.service.storage.snapshot().unwrap().revision, 0);
}
#[test]
#[cfg(unix)]
fn skills_v2_containment_dispatch_rejects_package_parent_replacement() {
    let f = Fixture::new();
    let record = f.install();
    let outside = f.root.join("workspace");
    fs::rename(f.home.join("skills"), &outside).unwrap();
    std::os::unix::fs::symlink(&outside, f.home.join("skills")).unwrap();
    assert!(
        f.turn().is_err(),
        "a matching digest does not authorize a root outside owned state"
    );
    assert!(outside
        .join("packages")
        .join(&record.id)
        .join(&record.digest)
        .join("SKILL.md")
        .is_file());
}
#[test]
fn skills_v2_containment_private_account_files_are_not_bundled_assets() {
    let f = Fixture::new();
    let source = f.root.join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("SKILL.md"), BODY).unwrap();
    // Synthetic non-secret content. Never read a real provider account here.
    fs::write(source.join("auth.json"), "{}").unwrap();
    assert!(
        f.service
            .prepare(&f.mutation(SkillInstallSource::Folder {
                path: source.to_string_lossy().into()
            }))
            .is_err(),
        "adjacent account/config files must not enter a skill payload"
    );
    assert_eq!(f.service.storage.snapshot().unwrap().revision, 0);
}
#[test]
fn skills_v2_containment_dispatch_rejects_modified_immutable_body() {
    let f = Fixture::new();
    let record = f.install();
    fs::write(
        f.service.package_root(&record).join("SKILL.md"),
        BODY.replace("documentation", "MUTATED documentation"),
    )
    .unwrap();
    assert!(f.turn().is_err());
}

#[test]
fn skills_v2_containment_repository_metadata_is_not_a_bundled_asset() {
    let f = Fixture::new();
    let source = f.root.join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("SKILL.md"), BODY).unwrap();
    fs::write(source.join(".git"), "gitdir: ../source-only-metadata\n").unwrap();
    let mutation = f.mutation(SkillInstallSource::Folder {
        path: source.to_string_lossy().into(),
    });
    let prepared = f.service.prepare(&mutation).unwrap();
    assert!(
        prepared[&0].1.files.iter().all(|file| file.path != ".git"),
        "repository bookkeeping is not a reusable skill asset"
    );
}
