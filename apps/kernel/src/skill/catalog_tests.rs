//! MP-08 / MP-10 / MP-11: source regressions, not SV real-path acceptance.
use super::{
    catalog::*, frontmatter::parse_frontmatter, resolver::*, selection::explicit_skill_names, v2::*,
};
use std::collections::BTreeMap;

fn skill(id: &str, name: &str, scope: SkillScope) -> SkillRecord {
    SkillRecord {
        id: id.into(),
        name: name.into(),
        scope,
        package_id: None,
        description: "Review the current code and explain concrete defects.".into(),
        short_description: None,
        default_state: SkillState::Auto,
        state_override: None,
        digest: "a".repeat(64),
        declared_version: None,
        files: vec![],
        provenance: SkillProvenance {
            kind: "content".into(),
            source_machine: None,
            locator: None,
            source_revision: None,
            provider: None,
            account_profile: None,
            installed_at_ms: 1,
            updated_at_ms: 1,
        },
        compatibility_notes: vec![],
    }
}
fn install(record: SkillRecord) -> (SkillCatalogMutation, BTreeMap<usize, SkillRecord>) {
    (
        SkillCatalogMutation {
            scope: record.scope.clone(),
            expected_revision: 0,
            actions: vec![SkillCatalogAction::Install {
                source: SkillInstallSource::Content {
                    skill_md: "unused prepared source".into(),
                },
                collision: SkillCollisionAction::Reject,
                package_id: None,
            }],
        },
        BTreeMap::from([(0, record)]),
    )
}
fn project(id: &str) -> SkillScope {
    SkillScope::Project {
        project_id: id.into(),
    }
}
fn apply(
    set: &SkillCatalogSet,
    scope: SkillScope,
    actions: Vec<SkillCatalogAction>,
) -> SkillCatalogSet {
    set.plan(
        &SkillCatalogMutation {
            scope,
            expected_revision: set.revision,
            actions,
        },
        &BTreeMap::new(),
    )
    .unwrap()
}
#[test]
fn skills_v2_project_catalogs_are_independent_of_folders_and_check_every_project() {
    let set = SkillCatalogSet::default();
    let (m, p) = install(skill("s1", "review", project("empty-project")));
    let set = set.plan(&m, &p).unwrap();
    assert_eq!(projections(&set, Some("empty-project"), &[]).len(), 1);
    assert!(projections(&set, Some("other-project"), &[]).is_empty());
    let (mut m, p) = install(skill("s2", "REVIEW", SkillScope::Chariox));
    m.expected_revision = set.revision;
    assert!(set
        .plan(&m, &p)
        .unwrap_err()
        .to_string()
        .contains("Rename or Replace"));
    // Project with no active session (including archived Projects) still collides globally.
    let (mut m, p) = install(skill("s2", "review", project("other-project")));
    m.expected_revision = set.revision;
    assert!(set.plan(&m, &p).is_ok());
}
#[test]
fn skills_v2_batches_are_atomic_and_require_expected_revision() {
    let (mut m, p) = install(skill("s1", "review", SkillScope::Chariox));
    m.actions.push(SkillCatalogAction::Regroup {
        skill_id: "s1".into(),
        package_id: Some("missing".into()),
    });
    let set = SkillCatalogSet::default();
    assert!(set.plan(&m, &p).is_err());
    assert!(set.record("s1").is_none());
    m.actions.pop();
    let set = set.plan(&m, &p).unwrap();
    assert_eq!(set.revision, 1);
    assert!(set.plan(&m, &p).is_err());
}
#[test]
fn skills_v2_replace_stays_in_original_scope_and_preserves_override() {
    let (m, p) = install(skill("s1", "review", SkillScope::Chariox));
    let set = SkillCatalogSet::default().plan(&m, &p).unwrap();
    let set = apply(
        &set,
        SkillScope::Chariox,
        vec![SkillCatalogAction::SetState {
            skill_id: "s1".into(),
            state: Some(SkillState::Manual),
        }],
    );
    let (mut m, p) = install(skill("s2", "review", project("p")));
    m.expected_revision = set.revision;
    m.actions[0] = SkillCatalogAction::Install {
        source: SkillInstallSource::Content {
            skill_md: "replacement".into(),
        },
        collision: SkillCollisionAction::Replace {
            skill_id: "s1".into(),
        },
        package_id: None,
    };
    let set = set.plan(&m, &p).unwrap();
    let row = set.record("s1").unwrap();
    assert_eq!(row.scope, SkillScope::Chariox);
    assert_eq!(row.state_override, Some(SkillState::Manual));
    assert!(set.record("s2").is_none());
}
#[test]
fn skills_v2_project_override_and_reset_follow_inherited_default() {
    let (m, p) = install(skill("s1", "review", SkillScope::Chariox));
    let set = SkillCatalogSet::default().plan(&m, &p).unwrap();
    let set = apply(
        &set,
        project("p"),
        vec![SkillCatalogAction::SetState {
            skill_id: "s1".into(),
            state: Some(SkillState::Off),
        }],
    );
    assert_eq!(
        projections(&set, Some("p"), &[])[0].effective_state,
        SkillState::Off
    );
    assert_eq!(
        projections(&set, Some("q"), &[])[0].effective_state,
        SkillState::Auto
    );
    let set = apply(
        &set,
        project("p"),
        vec![SkillCatalogAction::SetState {
            skill_id: "s1".into(),
            state: None,
        }],
    );
    assert_eq!(
        projections(&set, Some("p"), &[])[0].effective_state,
        SkillState::Auto
    );
    let set = apply(
        &set,
        SkillScope::Chariox,
        vec![SkillCatalogAction::Remove {
            skill_id: "s1".into(),
        }],
    );
    assert!(set
        .catalog(&project("p"))
        .unwrap()
        .chariox_overrides
        .is_empty());
}
#[test]
fn skills_v2_custom_regrouping_keeps_identity_and_unknown_provenance_ungrouped() {
    let (m, p) = install(skill("s1", "review", SkillScope::Chariox));
    let set = SkillCatalogSet::default().plan(&m, &p).unwrap();
    assert!(set.record("s1").unwrap().package_id.is_none());
    let set = apply(
        &set,
        SkillScope::Chariox,
        vec![
            SkillCatalogAction::CreatePackage {
                package_id: "p1".into(),
                name: "Source".into(),
            },
            SkillCatalogAction::CreatePackage {
                package_id: "p2".into(),
                name: "Custom".into(),
            },
            SkillCatalogAction::Regroup {
                skill_id: "s1".into(),
                package_id: Some("p1".into()),
            },
            SkillCatalogAction::Regroup {
                skill_id: "s1".into(),
                package_id: Some("p2".into()),
            },
        ],
    );
    assert_eq!(set.record("s1").unwrap().package_id.as_deref(), Some("p2"));
    assert_eq!(set.record("s1").unwrap().digest, "a".repeat(64));
    let set = apply(
        &set,
        SkillScope::Chariox,
        vec![SkillCatalogAction::RemovePackage {
            package_id: "p1".into(),
            remove_skills: true,
        }],
    );
    assert!(set.record("s1").is_some());
    let set = apply(
        &set,
        SkillScope::Chariox,
        vec![SkillCatalogAction::RemovePackage {
            package_id: "p2".into(),
            remove_skills: false,
        }],
    );
    assert!(set.record("s1").unwrap().package_id.is_none());
}
#[test]
fn skills_v2_move_is_atomic_and_clears_package_membership() {
    let (m, p) = install(skill("s1", "review", SkillScope::Chariox));
    let set = SkillCatalogSet::default().plan(&m, &p).unwrap();
    let set = apply(
        &set,
        SkillScope::Chariox,
        vec![SkillCatalogAction::Move {
            skill_id: "s1".into(),
            destination: project("p"),
        }],
    );
    assert_eq!(set.record("s1").unwrap().scope, project("p"));
    assert!(set.catalog(&SkillScope::Chariox).unwrap().skills.is_empty());
}
#[test]
fn skills_v2_yaml_has_typed_boolean_multiline_and_no_authority() {
    let skill=parse_frontmatter("---\nname: review\ndescription: |\n  First line\n  Second line\ndisable-model-invocation: true\nallowed-tools: [Bash]\n---\n!`echo no`\n").unwrap();
    assert_eq!(skill.default_state, SkillState::Manual);
    assert_eq!(skill.description, "First line\nSecond line\n");
    assert_eq!(skill.compatibility_notes.len(), 2);
    for value in ["'true'", "off", "1", "null"] {
        assert!(parse_frontmatter(&format!(
            "---\nname: review\ndescription: ok\ndisable-model-invocation: {value}\n---\n"
        ))
        .is_err());
    }
}
#[test]
fn skills_v2_dollar_parser_excludes_shell_code_quotes_attachments_and_currency() {
    let text="Use $review and $other. $HOME costs $42. \\$escaped `do $code` \"$quoted\" [$attachment](file:///x)\n```sh\n$in_fence\n```\n    $indented\n$review $MixedCase\n`multi\n$inside\nline` $after";
    assert_eq!(
        explicit_skill_names(text),
        vec!["review", "other", "MixedCase", "after"]
    );
    assert!(explicit_skill_names("@review review /skill review").is_empty());
    assert_eq!(
        explicit_skill_names("Let's use $review [a $hidden ](https://example.com)"),
        vec!["review"]
    );
}
#[test]
fn skills_v2_model_load_cannot_admit_manual_or_off() {
    let mut s = skill("s1", "review", SkillScope::Chariox);
    s.default_state = SkillState::Manual;
    let (m, p) = install(s);
    let set = SkillCatalogSet::default().plan(&m, &p).unwrap();
    let rows = projections(&set, None, &[]);
    assert!(admit_load(&rows, "s1", &[]).is_err());
    assert!(admit_load(&rows, "s1", &["s1".into()]).is_ok());
    let set = apply(
        &set,
        SkillScope::Chariox,
        vec![SkillCatalogAction::SetState {
            skill_id: "s1".into(),
            state: Some(SkillState::Off),
        }],
    );
    let rows = projections(&set, None, &[]);
    assert!(resolve_name(&rows, "review").is_err());
    assert!(admit_load(&rows, "s1", &["s1".into()]).is_err());
}
fn native(digest: &str) -> NativeSkillEntry {
    NativeSkillEntry {
        selection: SkillSelection::Native {
            entry_id: "n1".into(),
            name: "review".into(),
            provider: "codex".into(),
            account_profile: "account1".into(),
            execution_id: "lease1".into(),
            inventory_revision: "r1".into(),
        },
        description: "native review".into(),
        digest: Some(digest.into()),
        exposure: NativeSkillExposure::Confirmed,
        default_state: SkillState::Auto,
        complete_manifest: true,
        supports_load_ack: true,
        discovery_error: None,
    }
}
#[test]
fn skills_v2_exact_native_duplicate_has_one_picker_row_and_no_duplicate_metadata() {
    let (m, p) = install(skill("s1", "review", SkillScope::Chariox));
    let set = SkillCatalogSet::default().plan(&m, &p).unwrap();
    let native = vec![native(&"a".repeat(64))];
    let rows = projections(&set, None, &native);
    assert_eq!(rows[0].also_in_provider.as_deref(), Some("codex"));
    assert!(picker_native_entries(&rows, &native).is_empty());
    assert!(metadata_context(&rows, Some(128_000), 1.0)
        .unwrap()
        .text
        .is_empty());
}
#[test]
fn skills_v2_different_assets_conflict_and_unconfirmed_exposure_never_suppresses() {
    let (m, p) = install(skill("s1", "review", SkillScope::Chariox));
    let set = SkillCatalogSet::default().plan(&m, &p).unwrap();
    let mut native = vec![native(&"b".repeat(64))];
    let rows = projections(&set, None, &native);
    assert!(matches!(rows[0].delivery, SkillDelivery::Conflict { .. }));
    assert_eq!(picker_native_entries(&rows, &native).len(), 1);
    native[0].exposure = NativeSkillExposure::Unverified;
    let rows = projections(&set, None, &native);
    assert!(matches!(rows[0].delivery, SkillDelivery::Unverified { .. }));
    assert!(!metadata_context(&rows, None, 1.0).unwrap().text.is_empty());
    native[0].exposure = NativeSkillExposure::Disabled;
    assert!(matches!(
        projections(&set, None, &native)[0].delivery,
        SkillDelivery::Chariox
    ));
}
#[test]
fn skills_v2_native_chips_are_bound_to_provider_profile_execution_and_inventory() {
    let n = native("a");
    let inventory = vec![n.clone()];
    assert!(
        validate_native_selection(&n.selection, &inventory, "codex", "account1", "lease1").is_ok()
    );
    for (p, a, e) in [
        ("claude", "account1", "lease1"),
        ("codex", "account2", "lease1"),
        ("codex", "account1", "lease2"),
    ] {
        assert!(validate_native_selection(&n.selection, &inventory, p, a, e).is_err());
    }
    assert!(validate_native_selection(&n.selection, &[], "codex", "account1", "lease1").is_err());
}
#[test]
fn skills_v2_sixty_skill_budget_and_revision_bound_paging() {
    let mut set = SkillCatalogSet::default();
    for n in 0..60 {
        let (mut m, p) = install(skill(
            &format!("s{n}"),
            &format!("skill-{n}"),
            SkillScope::Chariox,
        ));
        m.expected_revision = set.revision;
        set = set.plan(&m, &p).unwrap();
    }
    let rows = projections(&set, None, &[]);
    let context = metadata_context(&rows, Some(8192), 1.0).unwrap();
    assert!(context.tokens_upper_bound <= 81);
    assert!(context.omitted_count > 0);
    let (first, cursor) = discovery_page(&rows, 60, None, 10).unwrap();
    assert_eq!(first.len(), 10);
    assert_eq!(
        discovery_page(&rows, 60, cursor.as_deref(), 10)
            .unwrap()
            .0
            .len(),
        10
    );
    assert!(discovery_page(&rows, 61, cursor.as_deref(), 10).is_err());
    for invalid in [0.0, -1.0, 101.0, f64::INFINITY, f64::NAN] {
        assert!(metadata_context(&rows, None, invalid).is_err());
    }
    assert!(metadata_context(&rows, None, 2.0).unwrap().budget_tokens > context.budget_tokens);
}
#[test]
fn skills_v2_safe_locators_remove_url_secrets() {
    assert_eq!(
        safe_locator("https://user:pass@example.com/skills?token=secret#fragment").unwrap(),
        "https://example.com/skills"
    );
}

struct StorageFixture {
    root: std::path::PathBuf,
    storage: super::storage::SkillCatalogStorage,
}
impl StorageFixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("skills-v2-storage-{:032x}", rand::random::<u128>()));
        std::fs::create_dir_all(&root).unwrap();
        let durable =
            crate::durable_state::DurableKernelStateStore::open(root.join("kernel.db")).unwrap();
        let storage = super::storage::SkillCatalogStorage::new(&root, durable).unwrap();
        Self { root, storage }
    }
}
impl Drop for StorageFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn skills_v2_durable_publication_rebuilds_lost_json_projections() {
    let f = StorageFixture::new();
    let (m, p) = install(skill("s1", "review", SkillScope::Chariox));
    let published = f.storage.publish(&m, &p).unwrap();
    assert_eq!(published.revision, 1);
    let path = f.root.join("state/skills/catalog.json");
    assert!(path.is_file());
    std::fs::remove_file(&path).unwrap();
    assert_eq!(f.storage.snapshot().unwrap(), published);
    assert_eq!(f.storage.recover_projections().unwrap(), published);
    let projection: SkillCatalog = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(projection.revision, 1);
    assert_eq!(projection.skills[0].id, "s1");
}
#[test]
fn skills_v2_concurrent_publication_rejects_one_stale_review() {
    let f = StorageFixture::new();
    let a = f.storage.clone();
    let b = f.storage.clone();
    let (ma, pa) = install(skill("s1", "one", SkillScope::Chariox));
    let (mb, pb) = install(skill("s2", "two", SkillScope::Chariox));
    let a = std::thread::spawn(move || a.publish(&ma, &pa));
    let b = std::thread::spawn(move || b.publish(&mb, &pb));
    let results = [a.join().unwrap(), b.join().unwrap()];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let snapshot = f.storage.snapshot().unwrap();
    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.catalogs[0].skills.len(), 1);
}
#[cfg(unix)]
#[test]
fn skills_v2_symlinked_state_is_rejected_before_publication() {
    let f = StorageFixture::new();
    std::fs::create_dir_all(f.root.join("state")).unwrap();
    std::fs::create_dir_all(f.root.join("unrelated")).unwrap();
    std::os::unix::fs::symlink(f.root.join("unrelated"), f.root.join("state/skills")).unwrap();
    let (m, p) = install(skill("s1", "review", SkillScope::Chariox));
    assert!(f.storage.publish(&m, &p).is_err());
    assert_eq!(f.storage.snapshot().unwrap().revision, 0);
    assert!(std::fs::read_dir(f.root.join("unrelated"))
        .unwrap()
        .next()
        .is_none());
}

fn service_install(
    f: &StorageFixture,
    body: &str,
) -> (super::service::SkillCatalogService, SkillCatalogSet) {
    let service = super::service::SkillCatalogService::new(f.storage.clone());
    let mutation = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: 0,
        actions: vec![SkillCatalogAction::Install {
            source: SkillInstallSource::Content {
                skill_md: body.into(),
            },
            collision: SkillCollisionAction::Reject,
            package_id: None,
        }],
    };
    let prepared = service.prepare(&mutation).unwrap();
    let published = service.publish_authorized(&mutation, &prepared).unwrap();
    (service, published)
}
#[test]
fn skills_v2_prepared_bytes_and_dispatch_revision_are_pinned() {
    let f = StorageFixture::new();
    let (service, set) = service_install(
        &f,
        "---\nname: review\ndescription: |\n  Review code carefully.\n---\nFIRST BODY\n",
    );
    let record = set.record(&set.catalogs[0].skills[0].id).unwrap();
    assert_eq!(
        service.package_root(record),
        f.root
            .join("skills/packages")
            .join(&record.id)
            .join(&record.digest)
    );
    let turn = service
        .pinned_turn(
            None,
            "agent1",
            "session1",
            "Use $review",
            &[],
            &[],
            Some(128_000),
            1.0,
            8192,
            "g1",
        )
        .unwrap();
    assert_eq!(turn.catalog_revision, 1);
    assert!(turn.loaded[0].body.contains("FIRST BODY"));
    let old_selection = SkillSelection::Chariox {
        skill_id: record.id.clone(),
        name: record.name.clone(),
        catalog_revision: 1,
    };
    let mutation = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: 1,
        actions: vec![SkillCatalogAction::Update {
            skill_id: record.id.clone(),
            expected_digest: record.digest.clone(),
            source: SkillInstallSource::Content {
                skill_md: "---\nname: review\ndescription: updated review\n---\nSECOND BODY\n"
                    .into(),
            },
        }],
    };
    let prepared = service.prepare(&mutation).unwrap();
    service.publish_authorized(&mutation, &prepared).unwrap();
    assert!(service
        .pinned_turn(
            None,
            "agent1",
            "session1",
            "continue",
            &[old_selection],
            &[],
            Some(128_000),
            1.0,
            8192,
            "g1"
        )
        .is_err());
    let turn = service
        .pinned_turn(
            None,
            "agent1",
            "session1",
            "$review",
            &[],
            &[],
            Some(128_000),
            1.0,
            8192,
            "g1",
        )
        .unwrap();
    assert_eq!(turn.catalog_revision, 2);
    assert!(turn.loaded[0].body.contains("SECOND BODY"));
}
#[test]
fn skills_v2_acknowledged_loads_restore_and_off_stops_future_delivery() {
    let f = StorageFixture::new();
    let (service, set) = service_install(
        &f,
        "---\nname: review\ndescription: review code\ndisable-model-invocation: true\n---\nBODY\n",
    );
    let record = &set.catalogs[0].skills[0];
    let receipt = SkillLoadRecord {
        skill_id: record.id.clone(),
        digest: record.digest.clone(),
        session_id: "session1".into(),
        agent_id: "agent1".into(),
        run_id: "run1".into(),
        prompt_id: "prompt1".into(),
        reason: SkillLoadReason::Explicit,
        context_generation: "g1".into(),
        delivered_at_ms: 1,
    };
    f.storage.record_load(&receipt).unwrap();
    f.storage.record_load(&receipt).unwrap();
    assert_eq!(f.storage.loads("session1").unwrap().len(), 1);
    let turn = service
        .pinned_turn(
            None,
            "agent1",
            "session1",
            "continue after reset",
            &[],
            &[],
            Some(128_000),
            1.0,
            8192,
            "g2",
        )
        .unwrap();
    assert_eq!(turn.loaded.len(), 1);
    assert_eq!(turn.context_generation, "g2");
    let mutation = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: 1,
        actions: vec![SkillCatalogAction::SetState {
            skill_id: record.id.clone(),
            state: Some(SkillState::Off),
        }],
    };
    f.storage.publish(&mutation, &BTreeMap::new()).unwrap();
    assert!(service
        .pinned_turn(
            None,
            "agent1",
            "session1",
            "continue",
            &[],
            &[],
            Some(128_000),
            1.0,
            8192,
            "g3"
        )
        .unwrap()
        .loaded
        .is_empty());
    assert_eq!(f.storage.loads("session1").unwrap().len(), 1);
}
#[test]
fn skills_v2_exact_duplicate_dispatch_contains_no_chariox_metadata_or_body() {
    let f = StorageFixture::new();
    let (service, set) = service_install(
        &f,
        "---\nname: review\ndescription: review code\n---\nBODY\n",
    );
    let record = &set.catalogs[0].skills[0];
    let inventory = vec![native(&record.digest)];
    let turn = service
        .pinned_turn(
            None,
            "agent1",
            "session1",
            "$review",
            &[],
            &inventory,
            Some(128_000),
            1.0,
            8192,
            "g1",
        )
        .unwrap();
    assert!(turn.metadata.is_empty());
    assert!(turn.loaded[0].body.is_empty());
    assert_eq!(turn.native_selections.len(), 1);
}

#[test]
fn skills_v2_equal_native_content_keeps_one_picker_row_across_independent_states() {
    let (m, p) = install(skill("s1", "review", SkillScope::Chariox));
    let set = SkillCatalogSet::default().plan(&m, &p).unwrap();
    let mut inventory = vec![native(&"a".repeat(64))];
    inventory[0].default_state = SkillState::Manual;
    let rows = projections(&set, None, &inventory);
    assert_eq!(rows[0].also_in_provider.as_deref(), Some("codex"));
    assert!(picker_native_entries(&rows, &inventory).is_empty());
    // Native Manual does not supply Auto discovery; only Chariox metadata is added.
    assert!(!metadata_context(&rows, Some(128_000), 1.0)
        .unwrap()
        .text
        .is_empty());
    let set = apply(
        &set,
        SkillScope::Chariox,
        vec![SkillCatalogAction::SetState {
            skill_id: "s1".into(),
            state: Some(SkillState::Off),
        }],
    );
    assert_eq!(
        picker_native_entries(&projections(&set, None, &inventory), &inventory).len(),
        1
    );
}
#[test]
fn skills_v2_missing_native_ack_fails_selection_without_duplicate_delivery() {
    let f = StorageFixture::new();
    let (service, set) = service_install(
        &f,
        "---\nname: review\ndescription: review code\n---\nBODY\n",
    );
    let mut inventory = vec![native(&set.catalogs[0].skills[0].digest)];
    inventory[0].supports_load_ack = false;
    let (_, rows) = service.projections(None, &inventory).unwrap();
    assert!(metadata_context(&rows, Some(128_000), 1.0)
        .unwrap()
        .text
        .is_empty());
    assert!(picker_native_entries(&rows, &inventory).is_empty());
    assert!(service
        .pinned_turn(
            None,
            "agent1",
            "session1",
            "$review",
            &[],
            &inventory,
            Some(128_000),
            1.0,
            8192,
            "g1"
        )
        .unwrap_err()
        .to_string()
        .contains("load acknowledgement"));
}
