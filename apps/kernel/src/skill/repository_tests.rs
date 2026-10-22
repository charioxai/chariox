//! MP-08 / MP-10 / MP-11: repository import against real git repositories on disk.
use super::{
    catalog_tests::StorageFixture, service::SkillCatalogService, source_access::SkillSourceAccess,
    v2::*,
};
use std::{fs, path::Path, process::Command};

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}
fn write_skill(root: &Path, path: &str, name: &str) {
    let dir = root.join(path);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Use for {name} work.\n---\nBody of {name}.\n"),
    )
    .unwrap();
}
struct Repo {
    root: std::path::PathBuf,
}
impl Repo {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("skills-v2-repo-{:032x}", rand::random::<u128>()));
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "--quiet", "-b", "main"]);
        write_skill(&root, "skills/alpha", "alpha");
        write_skill(&root, "skills/beta", "beta");
        fs::write(root.join("skills/beta/notes.txt"), "beta notes").unwrap();
        write_skill(&root, "skills/alpha/inner", "inner");
        fs::create_dir_all(root.join("skills/bad")).unwrap();
        fs::write(root.join("skills/bad/SKILL.md"), "no frontmatter").unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("docs/README.md"), "docs").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "--quiet", "-m", "first"]);
        // A non-tip commit must stay fetchable by id (hosting services allow it).
        git(&root, &["config", "uploadpack.allowAnySHA1InWant", "true"]);
        Self { root }
    }
    fn url(&self) -> String {
        url::Url::from_directory_path(&self.root)
            .unwrap()
            .to_string()
    }
    fn head(&self) -> String {
        git(&self.root, &["rev-parse", "HEAD"])
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn install_all(repo: &Repo, revision: &str, paths: &[&str]) -> SkillCatalogMutation {
    let mut actions = vec![SkillCatalogAction::CreatePackage {
        package_id: "package-repo".into(),
        name: "repo".into(),
    }];
    for path in paths {
        actions.push(SkillCatalogAction::Install {
            source: SkillInstallSource::Repository {
                url: repo.url(),
                revision: revision.into(),
                path: (*path).into(),
            },
            collision: SkillCollisionAction::Reject,
            package_id: Some("package-repo".into()),
        });
    }
    SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: 0,
        actions,
    }
}

#[test]
fn resolving_a_repository_lists_every_skill_directory_at_its_current_commit() {
    let f = StorageFixture::new();
    let repo = Repo::new();
    let service = SkillCatalogService::new(f.storage.clone());
    let SkillsV2Response::Repository {
        revision,
        skills,
        skipped,
        locator,
    } = service.resolve_repository(&repo.url()).unwrap()
    else {
        panic!("expected a repository listing");
    };
    assert_eq!(revision, repo.head());
    assert_eq!(locator, repo.url());
    let paths: Vec<_> = skills.iter().map(|s| s.path.as_str()).collect();
    // A skill directory owns what is below it: `inner` is not a second skill.
    assert_eq!(paths, ["skills/alpha", "skills/beta"]);
    assert_eq!(skills[0].name, "alpha");
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].path, "skills/bad");
    assert!(
        fs::read_dir(f.root.join("state/skill-staging")).map_or(true, |mut d| d.next().is_none()),
        "the checkout is removed"
    );
}

#[test]
fn repository_install_is_one_source_backed_package_at_the_attested_revision() {
    let f = StorageFixture::new();
    let repo = Repo::new();
    let service = SkillCatalogService::new(f.storage.clone());
    let revision = repo.head();
    let mutation = install_all(&repo, &revision, &["skills/alpha", "skills/beta"]);
    // The path a confirmed review takes: preview stages the bytes, publish reads them back.
    let SkillsV2Response::Preview { review_receipt, .. } =
        service.preview_review("owner", &mutation).unwrap()
    else {
        panic!("expected a review");
    };
    assert!(review_receipt
        .sources
        .iter()
        .all(|source| source.source_revision.as_deref() == Some(revision.as_str())));
    let set = service
        .publish_reviewed("owner", &mutation, &review_receipt.id)
        .unwrap();
    let catalog = set.catalog(&SkillScope::Chariox).unwrap();
    assert_eq!(catalog.packages.len(), 1);
    let package = &catalog.packages[0];
    assert!(!package.custom);
    assert_eq!(package.source_revision.as_deref(), Some(revision.as_str()));
    assert_eq!(package.source_locator.as_deref(), Some(repo.url().as_str()));
    let by_path: Vec<_> = package.upstream_members.keys().cloned().collect();
    assert_eq!(by_path, ["skills/alpha", "skills/beta"]);
    for skill in &catalog.skills {
        assert_eq!(skill.package_id.as_deref(), Some("package-repo"));
        assert_eq!(skill.provenance.kind, "repository");
        assert_eq!(
            skill.provenance.source_revision.as_deref(),
            Some(revision.as_str())
        );
        assert!(package.upstream_members.values().any(|id| id == &skill.id));
    }
    let beta = catalog.skills.iter().find(|s| s.name == "beta").unwrap();
    assert!(beta.files.iter().any(|f| f.path == "notes.txt"));
}

#[test]
fn repository_install_stays_pinned_when_the_repository_moves_on() {
    let f = StorageFixture::new();
    let repo = Repo::new();
    let service = SkillCatalogService::new(f.storage.clone());
    let reviewed = repo.head();
    fs::write(
        repo.root.join("skills/alpha/SKILL.md"),
        "---\nname: alpha\ndescription: Changed.\n---\nNew.\n",
    )
    .unwrap();
    git(&repo.root, &["commit", "--quiet", "-am", "second"]);
    let mutation = install_all(&repo, &reviewed, &["skills/alpha"]);
    let prepared = service.prepare(&mutation).unwrap();
    let (record, _) = &prepared[&1];
    assert_eq!(record.description, "Use for alpha work.");
    assert_eq!(
        record.provenance.source_revision.as_deref(),
        Some(reviewed.as_str())
    );
}

#[test]
fn repository_install_refuses_unknown_revisions_agents_and_credentials() {
    let f = StorageFixture::new();
    let repo = Repo::new();
    let service = SkillCatalogService::new(f.storage.clone());
    let unknown = install_all(&repo, &"ab".repeat(20), &["skills/alpha"]);
    assert!(service.prepare(&unknown).is_err());
    let short = install_all(&repo, "main", &["skills/alpha"]);
    assert!(service
        .prepare(&short)
        .unwrap_err()
        .to_string()
        .contains("full commit id"));
    let agent = install_all(&repo, &repo.head(), &["skills/alpha"]);
    let error = service
        .prepare_with_source_access(
            &agent,
            &Default::default(),
            SkillSourceAccess::Agent { workspace: None },
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("owner-initiated"), "{error}");
    let error = service
        .resolve_repository("https://user:secret@example.com/org/skills.git")
        .unwrap_err()
        .to_string();
    assert!(error.contains("only public repositories"), "{error}");
    assert!(!error.contains("secret"));
    assert!(
        fs::read_dir(f.root.join("state/skill-staging")).map_or(true, |mut d| d.next().is_none()),
        "a refused repository leaves no staging directory"
    );
}

#[test]
fn repository_install_rejects_a_missing_member_directory_and_publishes_nothing() {
    let f = StorageFixture::new();
    let repo = Repo::new();
    let service = SkillCatalogService::new(f.storage.clone());
    let mutation = install_all(
        &repo,
        &repo.head(),
        &["skills/alpha", "skills/missing", "../etc"],
    );
    assert!(service.prepare(&mutation).is_err());
    assert_eq!(f.storage.snapshot().unwrap().revision, 0);
}

#[test]
fn a_repository_package_refuses_members_from_another_commit_or_a_custom_package() {
    let f = StorageFixture::new();
    let repo = Repo::new();
    let service = SkillCatalogService::new(f.storage.clone());
    let first = repo.head();
    let mutation = install_all(&repo, &first, &["skills/alpha"]);
    let prepared = service.prepare(&mutation).unwrap();
    service.publish_authorized(&mutation, &prepared).unwrap();
    fs::write(repo.root.join("docs/more.md"), "x").unwrap();
    git(&repo.root, &["add", "."]);
    git(&repo.root, &["commit", "--quiet", "-m", "second"]);
    let second = repo.head();
    let mut later = install_all(&repo, &second, &["skills/beta"]);
    later.expected_revision = 1;
    later.actions.remove(0);
    let error = service.prepare(&later).unwrap_err().to_string();
    assert!(error.contains("same repository commit"), "{error}");
}

#[test]
fn importing_an_installed_repository_again_says_how_to_proceed() {
    let f = StorageFixture::new();
    let repo = Repo::new();
    let service = SkillCatalogService::new(f.storage.clone());
    let first = install_all(&repo, &repo.head(), &["skills/alpha"]);
    let prepared = service.prepare(&first).unwrap();
    service.publish_authorized(&first, &prepared).unwrap();
    let mut again = install_all(&repo, &repo.head(), &["skills/alpha"]);
    again.expected_revision = 1;
    again.actions[0] = SkillCatalogAction::CreatePackage {
        package_id: "package-again".into(),
        name: "again".into(),
    };
    again.actions[1] = SkillCatalogAction::Install {
        source: SkillInstallSource::Repository {
            url: repo.url(),
            revision: repo.head(),
            path: "skills/alpha".into(),
        },
        collision: SkillCollisionAction::Reject,
        package_id: Some("package-again".into()),
    };
    let error = service.prepare(&again).unwrap_err().to_string();
    assert!(error.contains("`alpha` is occupied"), "{error}");
    assert!(!error.contains("choose Rename or Replace"), "{error}");
    assert!(error.contains("leave that directory out"), "{error}");
}

fn update_members(
    repo: &Repo,
    revision: &str,
    catalog: &super::catalog::SkillCatalogSet,
) -> SkillCatalogMutation {
    let package = &catalog.catalogs[0].packages[0];
    SkillCatalogMutation {
        scope: package.scope.clone(),
        expected_revision: catalog.revision,
        actions: package
            .upstream_members
            .iter()
            .filter_map(|(path, id)| {
                let record = catalog.record(id)?;
                (record.package_id.as_deref() == Some(package.id.as_str())).then(|| {
                    SkillCatalogAction::Update {
                        skill_id: id.clone(),
                        expected_digest: record.digest.clone(),
                        source: SkillInstallSource::Repository {
                            url: repo.url(),
                            revision: revision.into(),
                            path: path.clone(),
                        },
                    }
                })
            })
            .collect(),
    }
}

// MP-08 / MP-10 / MP-11: source-backed packages advance as one reviewed revision.
#[test]
fn repository_update_preserves_member_ids_overrides_and_advances_unchanged_members() {
    let fixture = StorageFixture::new();
    let service = SkillCatalogService::new(fixture.storage.clone());
    let repo = Repo::new();
    let install = install_all(&repo, &repo.head(), &["skills/alpha", "skills/beta"]);
    let prepared = service.prepare(&install).unwrap();
    let catalog = service.publish_authorized(&install, &prepared).unwrap();
    let alpha = catalog.catalogs[0]
        .skills
        .iter()
        .find(|s| s.name == "alpha")
        .unwrap()
        .clone();
    let state = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: catalog.revision,
        actions: vec![SkillCatalogAction::SetState {
            skill_id: alpha.id.clone(),
            state: Some(SkillState::Manual),
        }],
    };
    let catalog = service
        .publish_authorized(&state, &service.prepare(&state).unwrap())
        .unwrap();
    fs::write(
        repo.root.join("skills/alpha/notes.txt"),
        "a new bundled reference",
    )
    .unwrap();
    git(&repo.root, &["add", "."]);
    git(&repo.root, &["commit", "--quiet", "-m", "second"]);
    let revision = repo.head();
    let update = update_members(&repo, &revision, &catalog);
    let prepared = service
        .prepare(&update)
        .expect("repository members must support normal reviewed updates");
    let updated = service.publish_authorized(&update, &prepared).unwrap();
    let current = updated.record(&alpha.id).unwrap();
    assert_ne!(current.digest, alpha.digest);
    assert_eq!(current.state_override, Some(SkillState::Manual));
    assert_eq!(
        current.provenance.installed_at_ms,
        alpha.provenance.installed_at_ms
    );
    assert_eq!(
        updated.catalogs[0].packages[0].source_revision.as_deref(),
        Some(revision.as_str())
    );
    assert!(updated.catalogs[0]
        .skills
        .iter()
        .all(|s| s.provenance.source_revision.as_deref() == Some(revision.as_str())));
    assert_eq!(
        updated.catalogs[0]
            .skills
            .iter()
            .map(|s| &s.id)
            .collect::<Vec<_>>(),
        catalog.catalogs[0]
            .skills
            .iter()
            .map(|s| &s.id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn repository_update_refuses_a_batch_that_leaves_one_member_at_an_old_commit() {
    let fixture = StorageFixture::new();
    let service = SkillCatalogService::new(fixture.storage.clone());
    let repo = Repo::new();
    let install = install_all(&repo, &repo.head(), &["skills/alpha", "skills/beta"]);
    let catalog = service
        .publish_authorized(&install, &service.prepare(&install).unwrap())
        .unwrap();
    fs::write(
        repo.root.join("skills/alpha/notes.txt"),
        "a new bundled reference",
    )
    .unwrap();
    git(&repo.root, &["add", "."]);
    git(&repo.root, &["commit", "--quiet", "-m", "second"]);
    let mut update = update_members(&repo, &repo.head(), &catalog);
    update.actions.truncate(1);
    let prepared = service.prepare(&update).unwrap();
    let error = service
        .publish_authorized(&update, &prepared)
        .unwrap_err()
        .to_string();
    assert!(error.contains("every remaining member"), "{error}");
    assert_eq!(service.storage.snapshot().unwrap(), catalog);
}

#[test]
fn repository_update_does_not_reclaim_a_member_moved_to_a_custom_package() {
    let fixture = StorageFixture::new();
    let service = SkillCatalogService::new(fixture.storage.clone());
    let repo = Repo::new();
    let install = install_all(&repo, &repo.head(), &["skills/alpha", "skills/beta"]);
    let catalog = service
        .publish_authorized(&install, &service.prepare(&install).unwrap())
        .unwrap();
    let beta = catalog.catalogs[0]
        .skills
        .iter()
        .find(|s| s.name == "beta")
        .unwrap()
        .clone();
    let regroup = SkillCatalogMutation {
        scope: SkillScope::Chariox,
        expected_revision: catalog.revision,
        actions: vec![
            SkillCatalogAction::CreatePackage {
                package_id: "package-custom".into(),
                name: "My skills".into(),
            },
            SkillCatalogAction::Regroup {
                skill_id: beta.id.clone(),
                package_id: Some("package-custom".into()),
            },
        ],
    };
    let catalog = service
        .publish_authorized(&regroup, &service.prepare(&regroup).unwrap())
        .unwrap();
    fs::write(
        repo.root.join("skills/beta/notes.txt"),
        "new upstream beta bytes",
    )
    .unwrap();
    git(&repo.root, &["add", "."]);
    git(&repo.root, &["commit", "--quiet", "-m", "second"]);
    let update = update_members(&repo, &repo.head(), &catalog);
    let updated = service
        .publish_authorized(&update, &service.prepare(&update).unwrap())
        .unwrap();
    let current = updated.record(&beta.id).unwrap();
    assert_eq!(current.package_id.as_deref(), Some("package-custom"));
    assert_eq!(current.digest, beta.digest);
    assert_eq!(current.provenance, beta.provenance);
}

#[test]
fn repository_update_stays_pinned_when_the_repository_moves_after_resolution() {
    let fixture = StorageFixture::new();
    let service = SkillCatalogService::new(fixture.storage.clone());
    let repo = Repo::new();
    let install = install_all(&repo, &repo.head(), &["skills/alpha"]);
    let catalog = service
        .publish_authorized(&install, &service.prepare(&install).unwrap())
        .unwrap();
    fs::write(
        repo.root.join("skills/alpha/notes.txt"),
        "reviewed second revision",
    )
    .unwrap();
    git(&repo.root, &["add", "."]);
    git(&repo.root, &["commit", "--quiet", "-m", "second"]);
    let update = update_members(&repo, &repo.head(), &catalog);
    fs::write(
        repo.root.join("skills/alpha/notes.txt"),
        "unreviewed third revision",
    )
    .unwrap();
    git(&repo.root, &["add", "."]);
    git(&repo.root, &["commit", "--quiet", "-m", "third"]);
    let prepared = service.prepare(&update).unwrap();
    let updated = service.publish_authorized(&update, &prepared).unwrap();
    let alpha = &updated.catalogs[0].skills[0];
    assert_eq!(
        fs::read_to_string(service.package_root(alpha).join("notes.txt")).unwrap(),
        "reviewed second revision"
    );
}

#[test]
fn repository_update_review_publishes_staged_bytes_without_refetching() {
    let fixture = StorageFixture::new();
    let service = SkillCatalogService::new(fixture.storage.clone());
    let repo = Repo::new();
    let install = install_all(&repo, &repo.head(), &["skills/alpha"]);
    let catalog = service
        .publish_authorized(&install, &service.prepare(&install).unwrap())
        .unwrap();
    fs::write(repo.root.join("skills/alpha/notes.txt"), "reviewed update").unwrap();
    git(&repo.root, &["add", "."]);
    git(&repo.root, &["commit", "--quiet", "-m", "update"]);
    let revision = repo.head();
    let update = update_members(&repo, &revision, &catalog);
    let SkillsV2Response::Preview { review_receipt, .. } =
        service.preview_review("owner", &update).unwrap()
    else {
        panic!("expected an update review");
    };
    fs::remove_dir_all(&repo.root).unwrap();
    let updated = service
        .publish_reviewed("owner", &update, &review_receipt.id)
        .unwrap();
    assert_eq!(
        updated.catalogs[0].packages[0].source_revision.as_deref(),
        Some(revision.as_str())
    );
    assert_eq!(updated.catalogs[0].skills[0].provenance.kind, "repository");
    assert_eq!(
        fs::read_to_string(
            service
                .package_root(&updated.catalogs[0].skills[0])
                .join("notes.txt")
        )
        .unwrap(),
        "reviewed update"
    );
}

#[test]
fn resolving_repository_update_reviews_changed_added_removed_and_unchanged_members() {
    let fixture = StorageFixture::new();
    let service = SkillCatalogService::new(fixture.storage.clone());
    let repo = Repo::new();
    write_skill(&repo.root, "skills/keep", "keep");
    git(&repo.root, &["add", "."]);
    git(&repo.root, &["commit", "--quiet", "-m", "keep"]);
    let install = install_all(
        &repo,
        &repo.head(),
        &["skills/alpha", "skills/beta", "skills/keep"],
    );
    let catalog = service
        .publish_authorized(&install, &service.prepare(&install).unwrap())
        .unwrap();
    fs::write(repo.root.join("skills/alpha/notes.txt"), "new reference").unwrap();
    fs::remove_dir_all(repo.root.join("skills/beta")).unwrap();
    write_skill(&repo.root, "skills/new", "new");
    git(&repo.root, &["add", "."]);
    git(&repo.root, &["commit", "--quiet", "-m", "update"]);
    let SkillsV2Response::RepositoryUpdate {
        changed,
        added,
        removed,
        unchanged,
        mutation,
        revision,
        ..
    } = service.resolve_repository_update("package-repo").unwrap()
    else {
        panic!("expected an update diff");
    };
    assert_eq!(
        changed
            .iter()
            .map(|s| s.member.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha"]
    );
    assert_eq!(
        added.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["new"]
    );
    assert_eq!(
        removed.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["beta"]
    );
    assert_eq!(
        unchanged
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["keep"]
    );
    assert_eq!(mutation.expected_revision, catalog.revision);
    assert_eq!(service.storage.snapshot().unwrap(), catalog);
    let SkillsV2Response::Preview { review_receipt, .. } =
        service.preview_review("owner", &mutation).unwrap()
    else {
        panic!("expected review")
    };
    let updated = service
        .publish_reviewed("owner", &mutation, &review_receipt.id)
        .unwrap();
    assert_eq!(
        updated.catalogs[0].packages[0].source_revision.as_deref(),
        Some(revision.as_str())
    );
    assert!(updated.catalogs[0].skills.iter().all(|s| s.name != "beta"));
}

#[test]
fn resolving_repository_update_keeps_unselected_members_out_and_rejects_invalid_current_members() {
    let fixture = StorageFixture::new();
    let service = SkillCatalogService::new(fixture.storage.clone());
    let repo = Repo::new();
    let install = install_all(&repo, &repo.head(), &["skills/alpha"]);
    let catalog = service
        .publish_authorized(&install, &service.prepare(&install).unwrap())
        .unwrap();
    fs::write(repo.root.join("docs/README.md"), "updated docs").unwrap();
    git(&repo.root, &["add", "."]);
    git(&repo.root, &["commit", "--quiet", "-m", "docs"]);
    let SkillsV2Response::RepositoryUpdate {
        added,
        skipped,
        mutation,
        ..
    } = service.resolve_repository_update("package-repo").unwrap()
    else {
        panic!("expected diff")
    };
    assert!(added.is_empty());
    assert!(skipped
        .iter()
        .any(|s| s.path == "skills/beta" && s.reason.contains("unselected")));
    assert_eq!(mutation.actions.len(), 1);
    fs::write(repo.root.join("skills/alpha/SKILL.md"), "invalid").unwrap();
    git(&repo.root, &["add", "."]);
    git(&repo.root, &["commit", "--quiet", "-m", "invalid"]);
    let error = service
        .resolve_repository_update("package-repo")
        .unwrap_err()
        .to_string();
    assert!(error.contains("invalid upstream"), "{error}");
    assert_eq!(service.storage.snapshot().unwrap(), catalog);
}
