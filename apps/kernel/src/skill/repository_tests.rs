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
