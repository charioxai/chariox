//! MP-08 / MP-10 / MP-11: source review integrity, not live SV acceptance.
use chariox_kernel::{
    durable_state::DurableKernelStateStore,
    skill::{service::SkillCatalogService, storage::SkillCatalogStorage, v2::*},
};
use std::{fs, path::PathBuf};
const FIRST: &str = "---\nname: review\ndescription: Review code\n---\nREVIEWED FIRST BODY\n";
const SECOND: &str = "---\nname: review\ndescription: Review code\n---\nUNREVIEWED SECOND BODY\n";
struct Fixture {
    root: PathBuf,
    home: PathBuf,
    source: PathBuf,
    service: SkillCatalogService,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("skills-v2-reviews-{:032x}", rand::random::<u128>()));
        let home = root.join("home");
        let source = root.join("source");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir(&source).unwrap();
        fs::write(source.join("SKILL.md"), FIRST).unwrap();
        let durable = DurableKernelStateStore::open(home.join("state.db")).unwrap();
        let service = SkillCatalogService::new(SkillCatalogStorage::new(&home, durable).unwrap());
        Self {
            root,
            home,
            source,
            service,
        }
    }
    fn mutation(&self) -> SkillCatalogMutation {
        SkillCatalogMutation {
            scope: SkillScope::Chariox,
            expected_revision: 0,
            actions: vec![SkillCatalogAction::Install {
                source: SkillInstallSource::Folder {
                    path: self.source.to_string_lossy().into(),
                },
                collision: SkillCollisionAction::Reject,
                package_id: None,
            }],
        }
    }
    fn preview(&self, mutation: &SkillCatalogMutation) -> SkillReviewReceipt {
        match self.service.preview_review("owner", mutation).unwrap() {
            SkillsV2Response::Preview { review_receipt, .. } => review_receipt,
            _ => panic!("expected reviewed mutation"),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn skills_v2_review_publishes_staged_bytes_after_source_changes() {
    let f = Fixture::new();
    let mutation = f.mutation();
    let receipt = f.preview(&mutation);
    assert_eq!(f.service.storage.snapshot().unwrap().revision, 0);
    fs::write(f.source.join("SKILL.md"), SECOND).unwrap();
    let published = f
        .service
        .publish_reviewed("owner", &mutation, &receipt.id)
        .unwrap();
    let skill = &published.catalogs[0].skills[0];
    assert_eq!(skill.digest, receipt.sources[0].digest);
    let body = fs::read_to_string(f.service.package_root(skill).join("SKILL.md")).unwrap();
    assert_eq!(
        body, FIRST,
        "approval cannot silently refetch newer source content"
    );
}
#[test]
fn skills_v2_review_is_bound_to_caller_and_exact_mutation() {
    let f = Fixture::new();
    let mutation = f.mutation();
    let receipt = f.preview(&mutation);
    assert!(f
        .service
        .publish_reviewed("collaborator", &mutation, &receipt.id)
        .is_err());
    let mut changed = mutation.clone();
    changed.scope = SkillScope::Project {
        project_id: "other-project".into(),
    };
    assert!(f
        .service
        .publish_reviewed("owner", &changed, &receipt.id)
        .is_err());
    let mut changed = mutation.clone();
    if let SkillCatalogAction::Install { collision, .. } = &mut changed.actions[0] {
        *collision = SkillCollisionAction::Rename {
            name: "different".into(),
        };
    }
    assert!(f
        .service
        .publish_reviewed("owner", &changed, &receipt.id)
        .is_err());
    assert_eq!(f.service.storage.snapshot().unwrap().revision, 0);
    assert!(f
        .service
        .publish_reviewed("owner", &mutation, &receipt.id)
        .is_ok());
}
#[test]
fn skills_v2_review_cannot_replay_or_overwrite_a_new_catalog_revision() {
    let f = Fixture::new();
    let mutation = f.mutation();
    let receipt = f.preview(&mutation);
    f.service
        .publish_reviewed("owner", &mutation, &receipt.id)
        .unwrap();
    assert!(f
        .service
        .publish_reviewed("owner", &mutation, &receipt.id)
        .is_err());
    let catalog = f.service.storage.snapshot().unwrap();
    assert_eq!(catalog.revision, 1);
    assert_eq!(catalog.catalogs[0].skills.len(), 1);
}
#[test]
fn skills_v2_review_corrupted_staging_never_publishes() {
    let f = Fixture::new();
    let mutation = f.mutation();
    let receipt = f.preview(&mutation);
    let body = f
        .home
        .join("state/skill-reviews")
        .join(&receipt.id)
        .join("member-0")
        .join(&receipt.sources[0].digest)
        .join("SKILL.md");
    fs::write(body, SECOND).unwrap();
    assert!(f
        .service
        .publish_reviewed("owner", &mutation, &receipt.id)
        .is_err());
    assert_eq!(f.service.storage.snapshot().unwrap().revision, 0);
    assert!(!f.home.join("skills/packages").exists());
}

#[test]
fn skills_v2_review_never_attests_a_caller_supplied_unresolved_revision() {
    let f = Fixture::new();
    let mut mutation = f.mutation();
    if let SkillCatalogAction::Install { source, .. } = &mut mutation.actions[0] {
        *source = SkillInstallSource::Url {
            url: "https://example.invalid/SKILL.md".into(),
            pinned_revision: Some("unresolved-caller-label".into()),
        };
    }
    let error = f.service.preview_review("owner", &mutation).unwrap_err();
    assert!(error.to_string().contains("cannot attest a revision"));
    assert_eq!(f.service.storage.snapshot().unwrap().revision, 0);
}
