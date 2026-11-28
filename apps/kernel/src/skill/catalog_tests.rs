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
// A Claude plugin skill is typed as `$plugin:skill`; the colon must not cut it to `$plugin`.
#[test]
fn skills_v2_dollar_parser_keeps_a_plugin_namespace() {
    assert_eq!(
        explicit_skill_names("Use $houseplug:house-lint now; $review: done, $tail:"),
        vec!["houseplug:house-lint", "review", "tail"]
    );
}
#[test]
fn skills_v2_namespaced_native_name_does_not_load_its_plugin_namesake() {
    let f = StorageFixture::new();
    let (service, _) = service_install(
        &f,
        "---\nname: houseplug\ndescription: unrelated\n---\nUNRELATED\n",
    );
    let mut entry = native(&"a".repeat(64));
    if let SkillSelection::Native { name, provider, .. } = &mut entry.selection {
        *name = "houseplug:house-lint".into();
        *provider = "claude".into();
    }
    let turn = service
        .pinned_turn(
            None,
            "agent1",
            "session1",
            "$houseplug:house-lint inspect source",
            &[],
            std::slice::from_ref(&entry),
            Some(128_000),
            1.0,
            8192,
            "g1",
        )
        .unwrap();
    assert!(turn.loaded.is_empty());
    assert_eq!(turn.native_selections, vec![entry.selection]);
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
    // A listed name finds the same skill as its id, so its state still decides.
    let error = admit_load(&rows, "Review", &[]).unwrap_err().to_string();
    assert!(error.contains("Off"), "{error}");
    let error = admit_load(&rows, "unknown", &[]).unwrap_err().to_string();
    assert!(error.contains("list_skills"), "{error}");
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
fn skills_v2_leased_worker_metadata_names_worker_roots_not_chariox_tools() {
    let mut set = SkillCatalogSet::default();
    let (mut m, p) = install(skill("s0", "skill-0", SkillScope::Chariox));
    m.expected_revision = set.revision;
    set = set.plan(&m, &p).unwrap();
    let rows = projections(&set, None, &[]);
    let roots = BTreeMap::from([(
        rows[0].skill.id.clone(),
        "/worker/exec/s0/digest".to_string(),
    )]);
    let text = crate::skill::resolver::metadata_context_at(&rows, None, 1.0, Some(&roots))
        .unwrap()
        .text;
    assert!(text.contains("/worker/exec/s0/digest/SKILL.md"));
    assert!(!text.contains("list_skills") && !text.contains("load_skill"));
    assert!(metadata_context(&rows, None, 1.0)
        .unwrap()
        .text
        .starts_with(
            "Skills: load a matching skill by name with chariox.load_skill before you act.\n"
        ));
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
fn skills_v2_metadata_lists_every_name_before_descriptions_and_points_to_paging() {
    let mut set = SkillCatalogSet::default();
    for n in 0..60 {
        let mut record = skill(&format!("s{n}"), &format!("skill-{n}"), SkillScope::Chariox);
        record.description =
            format!("Detailed guidance for area {n} with checks and fixes. ").repeat(8);
        let (mut m, p) = install(record);
        m.expected_revision = set.revision;
        set = set.plan(&m, &p).unwrap();
    }
    let rows = projections(&set, None, &[]);
    // A long first description must not crowd the later names out of discovery.
    let roomy = metadata_context(&rows[..10], Some(128_000), 1.0).unwrap();
    assert_eq!(roomy.omitted_count, 0);
    let names = |text: &str, rows: &[SkillProjection]| {
        rows.iter()
            .filter(|row| text.contains(&format!("\n{}:", row.skill.name)))
            .count()
    };
    assert_eq!(names(&roomy.text, &rows[..10]), 10);
    assert!(!roomy.text.contains("not listed"));
    // 60 names alone overflow 1% of a 64K window.
    let crowded = metadata_context(&rows, Some(64_000), 1.0).unwrap();
    assert!(crowded.omitted_count > 0);
    assert!(crowded.tokens_upper_bound <= crowded.budget_tokens);
    assert!(crowded.text.ends_with(&format!(
        "{} more Auto skills are not listed here; page through them with chariox.list_skills.\n",
        crowded.omitted_count
    )));
    assert_eq!(names(&crowded.text, &rows) + crowded.omitted_count, 60);
    let roots: BTreeMap<_, _> = rows
        .iter()
        .map(|row| (row.skill.id.clone(), format!("/worker/{}", row.skill.id)))
        .collect();
    let worker = metadata_context_at(&rows, Some(64_000), 1.0, Some(&roots)).unwrap();
    assert!(worker.text.ends_with(&format!(
        "{} more Auto skills are not listed here.\n",
        worker.omitted_count
    )));
}
#[test]
fn skills_v2_default_budget_lists_forty_names_with_whole_short_descriptions() {
    let mut set = SkillCatalogSet::default();
    for n in 0..40 {
        let mut record = skill(
            &format!("skill-{n:032x}"),
            &format!("topic-area-{n:02}"),
            SkillScope::Chariox,
        );
        record.description = if n == 17 {
            "Use when the user asks how to abort a browser fetch request.".into()
        } else {
            format!("Use when the user works on area {n}: failure modes and checks. ").repeat(4)
        };
        let (mut m, p) = install(record);
        m.expected_revision = set.revision;
        set = set.plan(&m, &p).unwrap();
    }
    let rows = projections(&set, None, &[]);
    // The default 1% of the unknown-window floor, as on a provider's first turn.
    let context = metadata_context(&rows, None, 1.0).unwrap();
    assert_eq!(context.omitted_count, 0, "{}", context.text);
    assert!(context.tokens_upper_bound <= context.budget_tokens);
    assert!(context.text.contains(
        "\ntopic-area-17: Use when the user asks how to abort a browser fetch request.\n"
    ));
    let row = admit_load(&rows, "Topic-Area-17", &[]).unwrap();
    assert_eq!(row.skill.id, format!("skill-{:032x}", 17));
}
#[test]
fn skills_v2_safe_locators_remove_url_secrets() {
    assert_eq!(
        safe_locator("https://user:pass@example.com/skills?token=secret#fragment").unwrap(),
        "https://example.com/skills"
    );
}

pub(super) struct StorageFixture {
    pub(super) root: std::path::PathBuf,
    pub(super) storage: super::storage::SkillCatalogStorage,
}
impl StorageFixture {
    pub(super) fn new() -> Self {
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
    // Ownership is pinned at construction. A later swap must also close reads,
    // rather than let a stale service publish through a replacement directory.
    std::fs::rename(f.root.join("state/skills"), f.root.join("original-skills")).unwrap();
    std::os::unix::fs::symlink(f.root.join("unrelated"), f.root.join("state/skills")).unwrap();
    let (m, p) = install(skill("s1", "review", SkillScope::Chariox));
    assert!(f.storage.publish(&m, &p).is_err());
    assert!(f.storage.snapshot().is_err());
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
// SV-14: an agent that guessed expected_revision must learn the current one from the refusal.
#[test]
fn skills_v2_stale_preview_names_the_current_revision() {
    let f = StorageFixture::new();
    let (service, published) =
        service_install(&f, "---\nname: one\ndescription: first skill\n---\nBODY\n");
    let stale = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: 0,
        actions: vec![SkillCatalogAction::Install {
            source: SkillInstallSource::Content {
                skill_md: "---\nname: two\ndescription: second skill\n---\nBODY\n".into(),
            },
            collision: SkillCollisionAction::Reject,
            package_id: None,
        }],
    };
    let error = service.prepare(&stale).unwrap_err().to_string();
    assert!(
        error.contains("preview again with expected_revision 1"),
        "{error}"
    );
    assert_eq!(f.storage.snapshot().unwrap(), published);
}
// Bundled examples are not secrets: a guide that shows PEM boundaries, an `.env.example`,
// a public certificate and a root `config.json` install like any other skill file.
#[test]
fn skills_v2_folder_install_keeps_harmless_secret_like_examples() {
    let f = StorageFixture::new();
    let folder = f.root.join("pem-guide");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("SKILL.md"),
        "---\nname: pem-guide\ndescription: Explain PEM files\n---\nA key file starts with\n-----BEGIN PRIVATE KEY-----\nand ends with -----END PRIVATE KEY-----.\n",
    )
    .unwrap();
    for (name, content) in [
        (".env.example", "API_URL=https://example.com\n"),
        (
            "cert.pem",
            "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n",
        ),
        ("config.json", "{\"lint\":true}\n"),
    ] {
        std::fs::write(folder.join(name), content).unwrap();
    }
    let service = super::service::SkillCatalogService::new(f.storage.clone());
    let mutation = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: 0,
        actions: vec![SkillCatalogAction::Install {
            source: SkillInstallSource::Folder {
                path: folder.display().to_string(),
            },
            collision: SkillCollisionAction::Reject,
            package_id: None,
        }],
    };
    let prepared = service.prepare(&mutation).unwrap();
    let published = service.publish_authorized(&mutation, &prepared).unwrap();
    let record = published
        .record(&published.catalogs[0].skills[0].id)
        .unwrap();
    let root = service.package_root(record);
    for name in ["SKILL.md", ".env.example", "cert.pem", "config.json"] {
        assert!(root.join(name).is_file(), "{name} must be installed");
    }
}
fn fresh_claude_run(model: &str) -> crate::provider::RuntimeProviderRun {
    crate::provider::RuntimeProviderRun::new(
        "fresh-run",
        &crate::provider::LaunchProviderRequest::new("s", "claude", "claude", "default", model),
        crate::provider::ProviderLaunchResult {
            endpoint_mode: crate::provider::AgentEndpointMode::Managed,
            process_label: "claude".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: None,
            structured_endpoint: None,
        },
    )
}

/// After a restart a fresh run carries the agent's rebuilt handoff (up to
/// 256,000 bytes) and reloads the skills it used before. Its model's window,
/// counted in tokens, leaves room for them; a 128K guess counted in bytes left none.
#[test]
fn a_fresh_run_reloads_a_skill_beside_a_full_handoff() {
    let f = StorageFixture::new();
    let body = "x".repeat(20_000);
    let (service, _) = service_install(
        &f,
        &format!("---\nname: big\ndescription: |\n  Large reference skill.\n---\n{body}\n"),
    );
    let run = fresh_claude_run("opus");
    assert_eq!(
        run.usage().context_window,
        None,
        "a fresh run reported no window yet"
    );
    let window = crate::provider::run_context_window_tokens(&run);
    assert_eq!(window, 1_000_000);
    let remaining = super::resolver::remaining_turn_budget(Some(window), None, 255_629);
    let turn = service
        .pinned_turn(
            None,
            "agent1",
            "session1",
            "Use $big",
            &[],
            &[],
            Some(window),
            1.0,
            remaining,
            "g1",
        )
        .unwrap();
    assert!(turn.loaded[0].body.len() >= 20_000);
    // A 200K model still fits its own (smaller) handoff and the skill.
    let window = crate::provider::run_context_window_tokens(&fresh_claude_run("claude-sonnet-4-5"));
    assert_eq!(window, 200_000);
    let remaining = super::resolver::remaining_turn_budget(Some(window), None, 90_000);
    assert!(remaining > 150_000);
}

#[test]
fn skills_v2_cold_start_explicit_body_fits_the_supported_context_floor() {
    let f = StorageFixture::new();
    let body = "x".repeat(24_000);
    let (service, _) = service_install(
        &f,
        &format!("---\nname: big\ndescription: |\n  Large reference skill.\n---\n{body}\n"),
    );
    // No provider usage yet: no window, no context tokens, 4 KB already carried.
    let remaining = super::resolver::remaining_turn_budget(None, None, 4_096);
    let turn = service
        .pinned_turn(
            None,
            "agent1",
            "session1",
            "Use $big",
            &[],
            &[],
            None,
            1.0,
            remaining,
            "g1",
        )
        .unwrap();
    assert!(turn.loaded[0].body.len() > 24_000);
    // A smaller reported window still bounds the same body.
    let small = super::resolver::remaining_turn_budget(Some(16_000), Some(1_000), 4_096);
    assert!(service
        .pinned_turn(
            None,
            "agent1",
            "session1",
            "Use $big",
            &[],
            &[],
            Some(16_000),
            1.0,
            small,
            "g1",
        )
        .is_err());
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
fn skills_v2_native_copy_without_ack_delivers_one_chariox_body() {
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
    assert_eq!(turn.loaded.len(), 1);
    assert_eq!(turn.loaded[0].delivery, SkillDelivery::Chariox);
    assert!(turn.loaded[0].body.contains("BODY"));
    assert!(
        turn.native_selections.is_empty(),
        "the native copy cannot acknowledge, so it must not also be invoked"
    );
}

#[test]
fn skills_v2_native_copy_with_ack_keeps_the_native_pass_through() {
    let f = StorageFixture::new();
    let (service, set) = service_install(
        &f,
        "---\nname: review\ndescription: review code\n---\nBODY\n",
    );
    let inventory = vec![native(&set.catalogs[0].skills[0].digest)];
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
    assert_eq!(turn.loaded.len(), 1);
    assert!(matches!(
        turn.loaded[0].delivery,
        SkillDelivery::ProvidedBy { .. }
    ));
    assert!(turn.loaded[0].body.is_empty());
    assert_eq!(turn.native_selections, vec![inventory[0].selection.clone()]);
}

#[test]
fn skills_v2_conflicting_native_copy_body_is_delivered_once_per_run() {
    let f = StorageFixture::new();
    let (service, _) = service_install(
        &f,
        "---\nname: review\ndescription: review code\n---\nCHARIOX BODY\n",
    );
    let inventory = vec![native(&"b".repeat(64))];
    let turn = |prompt: &str| {
        service
            .pinned_turn_from_snapshot(
                &f.storage.snapshot().unwrap(),
                None,
                "a",
                "s",
                prompt,
                &[],
                &inventory,
                Some(128_000),
                1.0,
                8192,
                "g",
                Some("run-1"),
            )
            .unwrap()
    };
    let first = turn("$review");
    assert_eq!(first.loaded.len(), 1);
    assert!(matches!(
        first.loaded[0].delivery,
        SkillDelivery::Conflict { .. }
    ));
    f.storage
        .record_load(&SkillLoadRecord {
            skill_id: first.loaded[0].skill_id.clone(),
            digest: first.loaded[0].digest.clone(),
            session_id: "s".into(),
            agent_id: "a".into(),
            run_id: "run-1".into(),
            prompt_id: "p".into(),
            reason: SkillLoadReason::Explicit,
            context_generation: "g".into(),
            delivered_at_ms: 1,
        })
        .unwrap();
    assert!(
        turn("later question").loaded.is_empty(),
        "the run already holds the body, and the conflict line rides only with a body"
    );
}

#[test]
fn skills_v2_review_explicit_native_selection_wins_over_conflicting_text_token() {
    let f = StorageFixture::new();
    let (service, _) = service_install(
        &f,
        "---\nname: review\ndescription: review code\n---\nCHARIOX BODY\n",
    );
    let inventory = vec![native(&"b".repeat(64))];
    let turn = service
        .pinned_turn(
            None,
            "a",
            "s",
            "$review",
            &[inventory[0].selection.clone()],
            &inventory,
            Some(128_000),
            1.0,
            8192,
            "g1",
        )
        .unwrap();
    assert!(
        turn.loaded.is_empty(),
        "From Codex must not also select the conflicting Chariox body"
    );
    assert_eq!(turn.native_selections, vec![inventory[0].selection.clone()]);
    // Plain text still chooses Chariox, preserving its normal precedence.
    let plain = service
        .pinned_turn(
            None,
            "a",
            "s",
            "$review",
            &[],
            &inventory,
            Some(128_000),
            1.0,
            8192,
            "g1",
        )
        .unwrap();
    assert_eq!(plain.loaded.len(), 1);
    assert!(plain.native_selections.is_empty());
}
#[test]
fn skills_v2_review_explicit_native_selection_is_independent_of_chariox_off() {
    let f = StorageFixture::new();
    let (service, set) = service_install(
        &f,
        "---\nname: review\ndescription: review code\n---\nCHARIOX BODY\n",
    );
    let id = set.catalogs[0].skills[0].id.clone();
    f.storage
        .publish(
            &SkillCatalogMutation {
                scope: SkillScope::Chariox,
                expected_revision: 1,
                actions: vec![SkillCatalogAction::SetState {
                    skill_id: id,
                    state: Some(SkillState::Off),
                }],
            },
            &BTreeMap::new(),
        )
        .unwrap();
    let inventory = vec![native(&"b".repeat(64))];
    let turn = service
        .pinned_turn(
            None,
            "a",
            "s",
            "$review",
            &[inventory[0].selection.clone()],
            &inventory,
            Some(128_000),
            1.0,
            8192,
            "g1",
        )
        .expect("explicit From Codex selection is independent of Chariox Off");
    assert!(turn.loaded.is_empty());
    assert_eq!(turn.native_selections.len(), 1);
    assert!(service
        .pinned_turn(
            None,
            "a",
            "s",
            "$review",
            &[],
            &inventory,
            Some(128_000),
            1.0,
            8192,
            "g1"
        )
        .is_err());
}
#[test]
fn skills_v2_review_manual_active_load_survives_other_agents_receipt_tail() {
    let f = StorageFixture::new();
    let (service, set) = service_install(&f, "---\nname: review\ndescription: review code\ndisable-model-invocation: true\n---\nMANUAL BODY\n");
    let skill = &set.catalogs[0].skills[0];
    let first = SkillLoadRecord {
        skill_id: skill.id.clone(),
        digest: skill.digest.clone(),
        session_id: "s".into(),
        agent_id: "original-agent".into(),
        run_id: "r".into(),
        prompt_id: "p".into(),
        reason: SkillLoadReason::Explicit,
        context_generation: "g1".into(),
        delivered_at_ms: 1,
    };
    f.storage.record_load(&first).unwrap();
    for n in 0..201 {
        let mut other = first.clone();
        other.agent_id = format!("other-agent-{n}");
        f.storage.record_load(&other).unwrap();
    }
    let turn = service
        .pinned_turn(
            None,
            "original-agent",
            "s",
            "continue after compaction",
            &[],
            &[],
            Some(128_000),
            1.0,
            8192,
            "g2",
        )
        .unwrap();
    assert_eq!(
        turn.loaded.len(),
        1,
        "an active Manual load is not a last-200 activity feed"
    );
    assert_eq!(turn.loaded[0].skill_id, skill.id);
    assert_eq!(f.storage.loads("s").unwrap().len(), 202);
    f.storage.record_load(&first).unwrap();
    assert_eq!(
        f.storage.loads("s").unwrap().len(),
        202,
        "old acknowledgement remains idempotent"
    );
}

#[test]
fn skills_v2_loaded_skill_is_delivered_once_per_provider_run() {
    let f = StorageFixture::new();
    let (service, set) = service_install(&f, "---\nname: review\ndescription: review code\ndisable-model-invocation: true\n---\nMANUAL BODY\n");
    let skill = &set.catalogs[0].skills[0];
    f.storage
        .record_load(&SkillLoadRecord {
            skill_id: skill.id.clone(),
            digest: skill.digest.clone(),
            session_id: "s".into(),
            agent_id: "a".into(),
            run_id: "run-1".into(),
            prompt_id: "p".into(),
            reason: SkillLoadReason::Explicit,
            context_generation: "g1".into(),
            delivered_at_ms: 1,
        })
        .unwrap();
    let turn = |prompt: &str, live_run_id: Option<&str>| {
        service
            .pinned_turn_from_snapshot(
                &f.storage.snapshot().unwrap(),
                None,
                "a",
                "s",
                prompt,
                &[],
                &[],
                Some(128_000),
                1.0,
                8192,
                "g2",
                live_run_id,
            )
            .unwrap()
    };
    assert!(
        turn("next question", Some("run-1")).loaded.is_empty(),
        "the same provider run already holds the body"
    );
    assert_eq!(
        turn("next question", None).loaded.len(),
        1,
        "a compaction or profile switch re-attaches the body"
    );
    assert_eq!(
        turn("next question", Some("run-2")).loaded.len(),
        1,
        "a restarted provider run has lost the body"
    );
    assert_eq!(
        turn("again $review", Some("run-1")).loaded.len(),
        1,
        "an explicit selection always delivers"
    );
    // The body was delivered again on the restarted run; the older record on the
    // previous run must not re-attach it on every later turn.
    f.storage
        .record_load(&SkillLoadRecord {
            skill_id: skill.id.clone(),
            digest: skill.digest.clone(),
            session_id: "s".into(),
            agent_id: "a".into(),
            run_id: "run-2".into(),
            prompt_id: "p2".into(),
            reason: SkillLoadReason::Explicit,
            context_generation: "g3".into(),
            delivered_at_ms: 2,
        })
        .unwrap();
    assert!(
        turn("next question", Some("run-2")).loaded.is_empty(),
        "the restarted run holds the body even though an older run's record exists"
    );
    // A changed body is not what the live run holds: it is delivered once, then held.
    let mutation = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: set.revision,
        actions: vec![SkillCatalogAction::Update {
            skill_id: skill.id.clone(),
            expected_digest: skill.digest.clone(),
            source: SkillInstallSource::Content {
                skill_md: "---\nname: review\ndescription: review code\ndisable-model-invocation: true\n---\nNEW BODY\n".into(),
            },
        }],
    };
    let prepared = service.prepare(&mutation).unwrap();
    let updated = service.publish_authorized(&mutation, &prepared).unwrap();
    let new_digest = updated.catalogs[0].skills[0].digest.clone();
    let changed = turn("next question", Some("run-2"));
    assert_eq!(changed.loaded.len(), 1, "a changed body is delivered once");
    assert!(changed.loaded[0].body.contains("NEW BODY"));
    f.storage
        .record_load(&SkillLoadRecord {
            skill_id: skill.id.clone(),
            digest: new_digest,
            session_id: "s".into(),
            agent_id: "a".into(),
            run_id: "run-2".into(),
            prompt_id: "p3".into(),
            reason: SkillLoadReason::Explicit,
            context_generation: "g4".into(),
            delivered_at_ms: 3,
        })
        .unwrap();
    assert!(
        turn("next question", Some("run-2")).loaded.is_empty(),
        "the live run holds the changed body; older records of the skill do not re-attach it"
    );
}

#[test]
fn skills_v2_review_model_load_becoming_manual_does_not_block_continuation() {
    let f = StorageFixture::new();
    let (service, set) = service_install(
        &f,
        "---\nname: review\ndescription: review code\n---\nBODY\n",
    );
    let skill = &set.catalogs[0].skills[0];
    let receipt = SkillLoadRecord {
        skill_id: skill.id.clone(),
        digest: skill.digest.clone(),
        session_id: "s".into(),
        agent_id: "a".into(),
        run_id: "r".into(),
        prompt_id: "p".into(),
        reason: SkillLoadReason::Model,
        context_generation: "g1".into(),
        delivered_at_ms: 1,
    };
    f.storage.record_load(&receipt).unwrap();
    f.storage
        .publish(
            &SkillCatalogMutation {
                scope: SkillScope::Chariox,
                expected_revision: set.revision,
                actions: vec![SkillCatalogAction::SetState {
                    skill_id: skill.id.clone(),
                    state: Some(SkillState::Manual),
                }],
            },
            &BTreeMap::new(),
        )
        .unwrap();
    for generation in ["g2", "after-reset"] {
        let turn = service
            .pinned_turn(
                None,
                "a",
                "s",
                "continue",
                &[],
                &[],
                Some(128_000),
                1.0,
                8192,
                generation,
            )
            .expect("past automatic delivery cannot block ordinary work after Manual");
        assert!(turn.loaded.is_empty());
        assert!(turn.metadata.is_empty());
    }
    let explicit = service
        .pinned_turn(
            None,
            "a",
            "s",
            "$review",
            &[],
            &[],
            Some(128_000),
            1.0,
            8192,
            "g4",
        )
        .unwrap();
    assert_eq!(explicit.loaded.len(), 1);
    assert_eq!(explicit.loaded[0].reason, SkillLoadReason::Explicit);
    f.storage
        .record_load(&SkillLoadRecord {
            reason: SkillLoadReason::Explicit,
            prompt_id: "p4".into(),
            context_generation: "g4".into(),
            delivered_at_ms: 4,
            ..receipt
        })
        .unwrap();
    let restored = service
        .pinned_turn(
            None,
            "a",
            "s",
            "continue",
            &[],
            &[],
            Some(128_000),
            1.0,
            8192,
            "g5",
        )
        .unwrap();
    assert_eq!(
        restored.loaded.len(),
        1,
        "an acknowledged explicit Manual load stays active"
    );
}

// MP-08 / MP-10 / MP-11: only resolved selections change ordinary prompt admission.
#[test]
fn skills_v2_unknown_ordinary_dollars_do_not_create_skill_invocations() {
    let f = StorageFixture::new();
    let (service, _) = service_install(
        &f,
        "---\nname: review\ndescription: Review code\n---\nReview instructions.\n",
    );
    let turn = service
        .pinned_turn(
            None,
            "agent",
            "session",
            "Why is $total null? Show $HOME, $20 and `$review`.",
            &[],
            &[],
            Some(128_000),
            1.0,
            8192,
            "generation",
        )
        .unwrap();
    assert!(turn.loaded.is_empty());
    assert!(turn.native_selections.is_empty());
    let stale = SkillSelection::Chariox {
        skill_id: "missing".into(),
        name: "total".into(),
        catalog_revision: turn.catalog_revision,
    };
    assert!(
        service
            .pinned_turn(
                None,
                "agent",
                "session",
                "Use $total",
                &[stale],
                &[],
                Some(128_000),
                1.0,
                8192,
                "generation"
            )
            .is_err(),
        "a source-bound stale selection must still refuse before a provider turn"
    );
}

#[test]
fn skills_v2_running_turn_catalog_stays_pinned_when_the_turn_publishes_an_update() {
    let f = StorageFixture::new();
    let (service, before) = service_install(
        &f,
        "---\nname: review\ndescription: Review code\n---\nORIGINAL TURN BODY\n",
    );
    let record = &before.catalogs[0].skills[0];
    let update = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: before.revision,
        actions: vec![SkillCatalogAction::Update {
            skill_id: record.id.clone(),
            expected_digest: record.digest.clone(),
            source: SkillInstallSource::Content {
                skill_md: "---\nname: review\ndescription: Updated review\n---\nNEXT TURN BODY\n"
                    .into(),
            },
        }],
    };
    let prepared = service.prepare(&update).unwrap();
    service.publish_authorized(&update, &prepared).unwrap();
    let pinned = service
        .storage
        .snapshot_at_revision(before.revision)
        .unwrap();
    assert_eq!(pinned, before);
    let old = service
        .pinned_turn_from_snapshot(
            &pinned,
            None,
            "agent",
            "session",
            "Use $review",
            &[],
            &[],
            Some(128_000),
            1.0,
            8192,
            "turn1",
            None,
        )
        .unwrap();
    assert!(old.loaded[0].body.contains("ORIGINAL TURN BODY"));
    let next = service
        .pinned_turn(
            None,
            "agent",
            "session",
            "Use $review",
            &[],
            &[],
            Some(128_000),
            1.0,
            8192,
            "turn2",
        )
        .unwrap();
    assert!(next.loaded[0].body.contains("NEXT TURN BODY"));
    assert!(next.catalog_revision > old.catalog_revision);
    assert!(service
        .storage
        .snapshot_at_revision(next.catalog_revision + 1)
        .is_err());
}

// MP-08 / MP-10 / MP-11: the feed is a session-scoped view of durable authority.
#[test]
fn skills_v2_watch_scopes_changes_and_pages_acknowledged_loads_without_a_new_store() {
    let f = StorageFixture::new();
    let (cursor, _) = f
        .storage
        .watch_changes("session", "project-a", Default::default())
        .unwrap();
    let (mutation, prepared) = install(skill("foreign-skill", "foreign", project("project-b")));
    f.storage.publish(&mutation, &prepared).unwrap();
    let (cursor, events) = f
        .storage
        .watch_changes("session", "project-a", cursor)
        .unwrap();
    assert_eq!(events.len(), 1);
    assert!(
        matches!(&events[0], SkillCatalogEvent::Changed { scope:SkillScope::Chariox, revision:1, changed_skill_ids, removed_skill_ids } if changed_skill_ids.is_empty() && removed_skill_ids.is_empty())
    );
    let (cursor, events) = f
        .storage
        .watch_changes("session", "project-a", cursor)
        .unwrap();
    assert!(events.is_empty());
    for index in 0..201 {
        f.storage
            .record_load(&SkillLoadRecord {
                skill_id: format!("skill-{index}"),
                digest: "a".repeat(64),
                session_id: "session".into(),
                agent_id: "agent".into(),
                run_id: "run".into(),
                prompt_id: "prompt".into(),
                reason: SkillLoadReason::Model,
                context_generation: "context".into(),
                delivered_at_ms: 1,
            })
            .unwrap();
    }
    f.storage
        .record_load(&SkillLoadRecord {
            skill_id: "other-session-skill".into(),
            digest: "b".repeat(64),
            session_id: "other-session".into(),
            agent_id: "agent".into(),
            run_id: "run".into(),
            prompt_id: "prompt".into(),
            reason: SkillLoadReason::Model,
            context_generation: "context".into(),
            delivered_at_ms: 1,
        })
        .unwrap();
    let (cursor, first) = f
        .storage
        .watch_changes("session", "project-a", cursor)
        .unwrap();
    assert_eq!(first.len(), 200);
    assert!(first.iter().all(|event| matches!(event, SkillCatalogEvent::Loaded { record } if record.session_id == "session")));
    let (cursor, second) = f
        .storage
        .watch_changes("session", "project-a", cursor)
        .unwrap();
    assert!(
        matches!(second.as_slice(), [SkillCatalogEvent::Loaded {record}] if record.skill_id == "skill-200")
    );
    assert!(f
        .storage
        .watch_changes("session", "project-a", cursor)
        .unwrap()
        .1
        .is_empty());
    assert_eq!(f.storage.loads("session").unwrap().len(), 201);
}

// MP-08 / MP-10 / MP-11: history is not continuing authority after revocation.
#[test]
fn skills_v2_manual_load_stays_revoked_after_off_and_reenable() {
    let f = StorageFixture::new();
    let (service, set) = service_install(
        &f,
        "---\nname: review\ndescription: review code\ndisable-model-invocation: true\n---\nBODY\n",
    );
    let skill = &set.catalogs[0].skills[0];
    let receipt = SkillLoadRecord {
        skill_id: skill.id.clone(),
        digest: skill.digest.clone(),
        session_id: "s".into(),
        agent_id: "a".into(),
        run_id: "r".into(),
        prompt_id: "p".into(),
        reason: SkillLoadReason::Explicit,
        context_generation: "g1".into(),
        delivered_at_ms: 1,
    };
    f.storage.record_load(&receipt).unwrap();
    for state in [SkillState::Off, SkillState::Manual] {
        f.storage
            .publish(
                &SkillCatalogMutation {
                    scope: SkillScope::Chariox,
                    expected_revision: f.storage.snapshot().unwrap().revision,
                    actions: vec![SkillCatalogAction::SetState {
                        skill_id: skill.id.clone(),
                        state: Some(state),
                    }],
                },
                &BTreeMap::new(),
            )
            .unwrap();
    }
    let turn = service
        .pinned_turn(
            None,
            "a",
            "s",
            "continue",
            &[],
            &[],
            Some(128_000),
            1.0,
            8192,
            "g2",
        )
        .unwrap();
    assert!(
        turn.loaded.is_empty(),
        "re-enabling Manual must not revive revoked instructions"
    );
    assert_eq!(
        f.storage.loads("s").unwrap(),
        [receipt],
        "delivery history remains visible"
    );
    let selected = service
        .pinned_turn(
            None,
            "a",
            "s",
            "$review",
            &[],
            &[],
            Some(128_000),
            1.0,
            8192,
            "g3",
        )
        .unwrap();
    assert_eq!(
        selected.loaded.len(),
        1,
        "a fresh explicit selection remains allowed"
    );
}

// MP-08 / MP-10 / MP-11: a late provider ACK is history, not new user authority.
#[test]
fn skills_v2_late_ack_cannot_cross_revocation_but_a_fresh_selection_can() {
    let f = StorageFixture::new();
    let (_, set) = service_install(
        &f,
        "---\nname: review\ndescription: review code\ndisable-model-invocation: true\n---\nBODY\n",
    );
    let skill = &set.catalogs[0].skills[0];
    let receipt = SkillLoadRecord {
        skill_id: skill.id.clone(),
        digest: skill.digest.clone(),
        session_id: "s".into(),
        agent_id: "a".into(),
        run_id: "r".into(),
        prompt_id: "p".into(),
        reason: SkillLoadReason::Explicit,
        context_generation: "old".into(),
        delivered_at_ms: 1,
    };
    for state in [SkillState::Off, SkillState::Manual] {
        f.storage
            .publish(
                &SkillCatalogMutation {
                    scope: SkillScope::Chariox,
                    expected_revision: f.storage.snapshot().unwrap().revision,
                    actions: vec![SkillCatalogAction::SetState {
                        skill_id: skill.id.clone(),
                        state: Some(state),
                    }],
                },
                &BTreeMap::new(),
            )
            .unwrap();
    }
    f.storage
        .record_load_at_revision(&receipt, Some("project-a"), 1)
        .unwrap();
    assert_eq!(f.storage.loads("s").unwrap().len(), 1);
    assert!(f
        .storage
        .active_loads("s", Some("project-a"), 3)
        .unwrap()
        .is_empty());
    let fresh = SkillLoadRecord {
        prompt_id: "new-prompt".into(),
        context_generation: "new".into(),
        ..receipt
    };
    f.storage
        .record_load_at_revision(&fresh, Some("project-a"), 3)
        .unwrap();
    assert_eq!(
        f.storage.active_loads("s", Some("project-a"), 3).unwrap(),
        std::slice::from_ref(&fresh)
    );
    assert!(f
        .storage
        .active_loads("s", Some("project-b"), 3)
        .unwrap()
        .is_empty());
    assert!(f
        .storage
        .record_load_at_revision(&fresh, Some("project-a"), 4)
        .is_err());
    assert_eq!(f.storage.loads("s").unwrap().len(), 2);
}

// MP-08 / MP-10 / MP-11: Project-local Off revokes only that Project's active copy.
#[test]
fn skills_v2_project_revocation_does_not_revoke_another_project_load() {
    let f = StorageFixture::new();
    let (_, set) = service_install(
        &f,
        "---\nname: review\ndescription: review code\ndisable-model-invocation: true\n---\nBODY\n",
    );
    let skill = &set.catalogs[0].skills[0];
    for project_id in ["project-a", "project-b"] {
        let receipt = SkillLoadRecord {
            skill_id: skill.id.clone(),
            digest: skill.digest.clone(),
            session_id: "s".into(),
            agent_id: "a".into(),
            run_id: "r".into(),
            prompt_id: project_id.into(),
            reason: SkillLoadReason::Explicit,
            context_generation: project_id.into(),
            delivered_at_ms: 1,
        };
        f.storage
            .record_load_at_revision(&receipt, Some(project_id), 1)
            .unwrap();
    }
    for state in [SkillState::Off, SkillState::Manual] {
        f.storage
            .publish(
                &SkillCatalogMutation {
                    scope: project("project-a"),
                    expected_revision: f.storage.snapshot().unwrap().revision,
                    actions: vec![SkillCatalogAction::SetState {
                        skill_id: skill.id.clone(),
                        state: Some(state),
                    }],
                },
                &BTreeMap::new(),
            )
            .unwrap();
    }
    assert!(f
        .storage
        .active_loads("s", Some("project-a"), 3)
        .unwrap()
        .is_empty());
    let other = f.storage.active_loads("s", Some("project-b"), 3).unwrap();
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].context_generation, "project-b");
    assert_eq!(f.storage.loads("s").unwrap().len(), 2);
}
// G-CELLS Q2: a queued `$review` chip picked at revision 1 starts its turn on a later
// catalog. Each change after queueing ends the turn with what changed and what to do next.
const QUEUED_REVIEW: &str =
    "---\nname: review\ndescription: review code\ndisable-model-invocation: true\n---\nBODY\n";
fn queued_review_turn(
    service: &super::service::SkillCatalogService,
    record: &SkillRecord,
) -> Result<SkillTurnContext, crate::error::DaemonError> {
    service.pinned_turn(
        None,
        "agent1",
        "session1",
        "continue",
        &[SkillSelection::Chariox {
            skill_id: record.id.clone(),
            name: record.name.clone(),
            catalog_revision: 1,
        }],
        &[],
        Some(128_000),
        1.0,
        8192,
        "g1",
    )
}
fn publish(
    service: &super::service::SkillCatalogService,
    revision: u64,
    actions: Vec<SkillCatalogAction>,
) {
    let mutation = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: revision,
        actions,
    };
    let prepared = service.prepare(&mutation).unwrap();
    service.publish_authorized(&mutation, &prepared).unwrap();
}
fn content_install(skill_md: &str) -> SkillCatalogAction {
    SkillCatalogAction::Install {
        source: SkillInstallSource::Content {
            skill_md: skill_md.into(),
        },
        collision: SkillCollisionAction::Reject,
        package_id: None,
    }
}
#[test]
fn skills_v2_queued_chip_turned_off_after_queueing_names_the_change() {
    let f = StorageFixture::new();
    let (service, set) = service_install(&f, QUEUED_REVIEW);
    let record = set.catalogs[0].skills[0].clone();
    publish(
        &service,
        1,
        vec![SkillCatalogAction::SetState {
            skill_id: record.id.clone(),
            state: Some(SkillState::Off),
        }],
    );
    let error = queued_review_turn(&service, &record)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("skill `review` was turned off after this prompt was queued; turn it back on in Skills or resend without it"),
        "{error}"
    );
}
#[test]
fn skills_v2_queued_chip_removed_after_queueing_names_the_change() {
    let f = StorageFixture::new();
    let (service, set) = service_install(&f, QUEUED_REVIEW);
    let record = set.catalogs[0].skills[0].clone();
    publish(
        &service,
        1,
        vec![SkillCatalogAction::Remove {
            skill_id: record.id.clone(),
        }],
    );
    let error = queued_review_turn(&service, &record)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("skill `review` was removed after this prompt was queued; reinstall it or resend without it"),
        "{error}"
    );
}
#[test]
fn skills_v2_queued_chip_replaced_after_queueing_names_the_change() {
    let f = StorageFixture::new();
    let (service, set) = service_install(&f, QUEUED_REVIEW);
    let record = set.catalogs[0].skills[0].clone();
    publish(
        &service,
        1,
        vec![SkillCatalogAction::Remove {
            skill_id: record.id.clone(),
        }],
    );
    publish(
        &service,
        2,
        vec![content_install(
            "---\nname: review\ndescription: another review\n---\nOTHER BODY\n",
        )],
    );
    let error = queued_review_turn(&service, &record)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("skill `review` was replaced after this prompt was queued; resend the prompt to use the new one"),
        "{error}"
    );
}
#[test]
fn skills_v2_queued_chip_with_a_stale_digest_names_the_change() {
    let f = StorageFixture::new();
    let (service, set) = service_install(&f, QUEUED_REVIEW);
    let record = set.catalogs[0].skills[0].clone();
    publish(
        &service,
        1,
        vec![SkillCatalogAction::Update {
            skill_id: record.id.clone(),
            expected_digest: record.digest.clone(),
            source: SkillInstallSource::Content {
                skill_md: "---\nname: review\ndescription: review code\ndisable-model-invocation: true\n---\nSECOND BODY\n".into(),
            },
        }],
    );
    let error = queued_review_turn(&service, &record)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("skill `review` changed after this prompt was queued; resend the prompt to use the current version"),
        "{error}"
    );
}
#[test]
fn skills_v2_queued_chip_whose_package_is_missing_at_turn_start_names_it() {
    let f = StorageFixture::new();
    let (service, set) = service_install(&f, QUEUED_REVIEW);
    let record = set.catalogs[0].skills[0].clone();
    std::fs::remove_dir_all(service.package_root(&record)).unwrap();
    let error = queued_review_turn(&service, &record)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("skill `review` package is missing or damaged on this machine; reinstall it in Skills, then resend the prompt"),
        "{error}"
    );
}
#[test]
fn skills_v2_queued_chip_survives_an_unrelated_catalog_change() {
    let f = StorageFixture::new();
    let (service, set) = service_install(&f, QUEUED_REVIEW);
    let record = set.catalogs[0].skills[0].clone();
    publish(
        &service,
        1,
        vec![content_install(
            "---\nname: lint\ndescription: lint code\n---\nLINT\n",
        )],
    );
    let turn = queued_review_turn(&service, &record).unwrap();
    assert_eq!(turn.catalog_revision, 2);
    assert_eq!(turn.loaded.len(), 1);
    assert!(turn.loaded[0].body.contains("BODY"));
}
