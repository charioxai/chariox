//! MP-08 / MP-10 / MP-11: competing kernels cannot replace home catalog authority.
//! Source regressions only; these do not establish live SV acceptance.
use chariox_kernel::{
    durable_state::DurableKernelStateStore,
    skill::{service::SkillCatalogService, storage::SkillCatalogStorage, v2::*},
};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Barrier},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("skills-v2-owner-{:032x}", rand::random::<u128>()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn journal(&self, name: &str) -> DurableKernelStateStore {
        DurableKernelStateStore::open(self.0.join(name).join("state.db")).unwrap()
    }
    fn mutation(&self) -> SkillCatalogMutation {
        SkillCatalogMutation {
            scope: SkillScope::Chariox,
            expected_revision: 0,
            actions: vec![SkillCatalogAction::Install {
                source: SkillInstallSource::Content { skill_md: "---\nname: review\ndescription: Review code\n---\nRead the source carefully.\n".into() },
                collision: SkillCollisionAction::Reject,
                package_id: None,
            }],
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn skills_v2_ownership_refuses_a_second_journal_before_it_can_replace_projections() {
    let f = Fixture::new();
    let first = SkillCatalogStorage::new(&f.0, f.journal("first")).unwrap();
    let service = SkillCatalogService::new(first.clone());
    let mutation = f.mutation();
    let prepared = service.prepare(&mutation).unwrap();
    let published = service.publish_authorized(&mutation, &prepared).unwrap();
    let projection = fs::read(f.0.join("state/skills/catalog.json")).unwrap();
    let second = SkillCatalogStorage::new(&f.0, f.journal("second"));
    assert!(
        second.is_err(),
        "a second journal must not become the same home catalog authority"
    );
    assert_eq!(first.snapshot().unwrap(), published);
    assert_eq!(
        fs::read(f.0.join("state/skills/catalog.json")).unwrap(),
        projection
    );
}

#[test]
fn skills_v2_ownership_concurrent_first_claims_select_exactly_one_existing_journal() {
    let f = Fixture::new();
    let first = f.journal("first");
    let second = f.journal("second");
    let gate = Arc::new(Barrier::new(2));
    let threads: Vec<_> = [first, second]
        .into_iter()
        .map(|journal| {
            let root = f.0.clone();
            let gate = gate.clone();
            std::thread::spawn(move || {
                gate.wait();
                SkillCatalogStorage::new(&root, journal)
            })
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "independent durable writers must not both claim one home catalog"
    );
}

#[test]
fn skills_v2_ownership_recovers_only_from_the_owning_journal_after_marker_loss() {
    let f = Fixture::new();
    let first = f.journal("first");
    let storage = SkillCatalogStorage::new(&f.0, first.clone()).unwrap();
    storage.recover_projections().unwrap();
    let _ = fs::remove_file(f.0.join("state/skills/home-journal.json"));
    assert!(
        SkillCatalogStorage::new(&f.0, f.journal("second")).is_err(),
        "missing projection metadata cannot authorize an empty foreign journal"
    );
    let restored = SkillCatalogStorage::new(&f.0, first).unwrap();
    assert_eq!(restored.snapshot().unwrap(), storage.snapshot().unwrap());
}

#[test]
fn skills_v2_ownership_reopening_the_same_journal_keeps_catalog_revision() {
    let f = Fixture::new();
    let path = f.0.join("first/state.db");
    let storage = SkillCatalogStorage::new(&f.0, f.journal("first")).unwrap();
    let service = SkillCatalogService::new(storage);
    let mutation = f.mutation();
    let prepared = service.prepare(&mutation).unwrap();
    let expected = service.publish_authorized(&mutation, &prepared).unwrap();
    drop(service);
    let restored =
        SkillCatalogStorage::new(&f.0, DurableKernelStateStore::open(path).unwrap()).unwrap();
    assert_eq!(restored.recover_projections().unwrap(), expected);
}
