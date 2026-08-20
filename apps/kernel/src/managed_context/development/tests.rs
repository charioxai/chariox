use super::*;
use flate2::read::GzDecoder;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn exports_two_repositories_with_unpushed_commits_and_exact_dirty_states() {
    let root = test_root("two-repositories");
    let primary = root.join("chariox");
    let supporting = root.join("chariox-cloud");
    init_repository(&primary, "primary.txt", "base\n");
    init_repository(&supporting, "supporting.txt", "support\n");
    git(
        &primary,
        &[
            "remote",
            "add",
            "origin",
            "https://user:secret@example.com/org/chariox.git",
        ],
    );
    fs::write(primary.join("unpushed.txt"), "unpushed\n").expect("write unpushed file");
    git(&primary, &["add", "unpushed.txt"]);
    git(&primary, &["commit", "-m", "unpushed"]);

    fs::write(primary.join("staged-and-working.txt"), "staged\n").expect("write staged file");
    git(&primary, &["add", "staged-and-working.txt"]);
    fs::write(primary.join("staged-and-working.txt"), "working\n").expect("write working file");
    fs::write(primary.join("untracked.txt"), "untracked\n").expect("write untracked file");
    fs::write(primary.join(".env.local"), "SECRET=not-transferred\n")
        .expect("write excluded environment file");
    fs::create_dir_all(primary.join(".chariox")).expect("create repository capability directory");
    fs::write(
        primary.join(".chariox/project.json"),
        "{\"enabled\":true}\n",
    )
    .expect("write repository capability");
    fs::write(primary.join("AGENTS.md"), "agent instructions\n").expect("write instructions");

    let archive_path = root.join("development-context.tar.gz");
    let result = export_development_context(DevelopmentContextExportRequest {
        project_id: "project-1".to_string(),
        repositories: vec![
            DevelopmentRepositorySelection {
                workspace_id: primary.display().to_string(),
                worktree_path: primary.clone(),
                role: DevelopmentRepositoryRole::Primary,
            },
            DevelopmentRepositorySelection {
                workspace_id: supporting.display().to_string(),
                worktree_path: supporting.clone(),
                role: DevelopmentRepositoryRole::Supporting,
            },
        ],
        archive_path: archive_path.clone(),
    })
    .expect("development context should export");

    assert_eq!(result.manifest.repositories.len(), 2);
    assert_eq!(
        result.manifest.repositories[0].role,
        DevelopmentRepositoryRole::Primary
    );
    assert_eq!(
        result.manifest.repositories[1].role,
        DevelopmentRepositoryRole::Supporting
    );
    assert_eq!(
        result.manifest.repositories[0].origin_url.as_deref(),
        Some("https://example.com/org/chariox.git")
    );
    assert_eq!(
        result.source_repositories[0].source_workspace_id,
        primary.display().to_string()
    );
    assert!(!serde_json::to_string(&result.manifest)
        .expect("manifest should serialize")
        .contains(&primary.display().to_string()));
    assert_eq!(
        sha256_file(&archive_path).expect("archive should hash"),
        result.archive_sha256
    );
    let repeated_archive_path = root.join("development-context-repeated.tar.gz");
    let repeated = export_development_context(DevelopmentContextExportRequest {
        project_id: "project-1".to_string(),
        repositories: vec![
            DevelopmentRepositorySelection {
                workspace_id: primary.display().to_string(),
                worktree_path: primary.clone(),
                role: DevelopmentRepositoryRole::Primary,
            },
            DevelopmentRepositorySelection {
                workspace_id: supporting.display().to_string(),
                worktree_path: supporting.clone(),
                role: DevelopmentRepositoryRole::Supporting,
            },
        ],
        archive_path: repeated_archive_path.clone(),
    })
    .expect("unchanged development context should export again");
    assert_eq!(repeated.archive_sha256, result.archive_sha256);
    assert_eq!(
        fs::read(repeated_archive_path).expect("read repeated archive"),
        fs::read(&archive_path).expect("read original archive")
    );
    assert_no_export_temporaries(&root);

    let unpacked = root.join("unpacked");
    unpack_archive(&archive_path, &unpacked);
    let primary_manifest = &result.manifest.repositories[0];
    let clone = root.join("bundle-clone");
    git_in(
        &root,
        &[
            "clone",
            unpacked
                .join(&primary_manifest.bundle_path)
                .to_str()
                .expect("bundle path"),
            clone.to_str().expect("clone path"),
        ],
    );
    assert_eq!(
        git_text_test(&clone, &["rev-parse", "HEAD"]),
        primary_manifest.head_sha
    );
    assert_eq!(
        fs::read_to_string(clone.join("unpushed.txt")).expect("unpushed file"),
        "unpushed\n"
    );

    let overlay = primary_manifest
        .overlay
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    assert!(!overlay.contains_key(".env.local"));
    assert!(overlay.contains_key(".chariox/project.json"));
    assert!(overlay.contains_key("AGENTS.md"));
    let staged = overlay
        .get("staged-and-working.txt")
        .expect("staged and working entry");
    assert_file_state_bytes(&unpacked, &staged.index, b"staged\n");
    assert_file_state_bytes(&unpacked, &staged.worktree, b"working\n");
    let untracked = overlay.get("untracked.txt").expect("untracked entry");
    assert_eq!(untracked.index, DevelopmentFileState::Absent);
    assert_file_state_bytes(&unpacked, &untracked.worktree, b"untracked\n");

    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn rejects_duplicate_sources_external_symlinks_submodules_and_lfs() {
    let root = test_root("unsafe-repositories");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");

    let duplicate = export_development_context(DevelopmentContextExportRequest {
        project_id: "project-duplicate".to_string(),
        repositories: vec![
            DevelopmentRepositorySelection {
                workspace_id: "one".to_string(),
                worktree_path: repository.clone(),
                role: DevelopmentRepositoryRole::Primary,
            },
            DevelopmentRepositorySelection {
                workspace_id: "two".to_string(),
                worktree_path: repository.clone(),
                role: DevelopmentRepositoryRole::Supporting,
            },
        ],
        archive_path: root.join("duplicate.tar.gz"),
    })
    .expect_err("duplicate source should fail");
    assert!(duplicate.to_string().contains("selected more than once"));

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/tmp/outside", repository.join("outside-link"))
            .expect("create external symlink");
        git(&repository, &["add", "outside-link"]);
        git(&repository, &["commit", "-m", "external symlink"]);
        let error = one_repo_export(&root, &repository, "external-symlink")
            .expect_err("external symlink should fail");
        assert!(error.to_string().contains("points outside the repository"));
        git(&repository, &["reset", "--hard", "HEAD~1"]);
    }

    let head = git_text_test(&repository, &["rev-parse", "HEAD"]);
    let cacheinfo = format!("160000,{head},vendor/dependency");
    git(
        &repository,
        &["update-index", "--add", "--cacheinfo", &cacheinfo],
    );
    git(&repository, &["commit", "-m", "submodule entry"]);
    let submodule =
        one_repo_export(&root, &repository, "submodule").expect_err("submodule should fail");
    assert!(submodule.to_string().contains("contains submodule"));
    git(&repository, &["reset", "--hard", "HEAD~1"]);

    fs::write(
        repository.join("lfs-pointer.bin"),
        "version https://git-lfs.github.com/spec/v1\noid sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nsize 1\n",
    )
    .expect("write LFS pointer");
    git(&repository, &["add", "lfs-pointer.bin"]);
    git(&repository, &["commit", "-m", "lfs pointer"]);
    let lfs = one_repo_export(&root, &repository, "lfs").expect_err("LFS should fail");
    assert!(lfs.to_string().contains("Git LFS pointers"));

    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn rejects_dirty_symlinks_and_requires_exactly_one_primary() {
    let root = test_root("selection-and-dirty-symlink");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");

    let missing_primary = export_development_context(DevelopmentContextExportRequest {
        project_id: "project-missing-primary".to_string(),
        repositories: vec![DevelopmentRepositorySelection {
            workspace_id: "workspace".to_string(),
            worktree_path: repository.clone(),
            role: DevelopmentRepositoryRole::Supporting,
        }],
        archive_path: root.join("missing-primary.tar.gz"),
    })
    .expect_err("missing primary should fail");
    assert!(missing_primary.to_string().contains("exactly one primary"));

    let occupied_archive = root.join("occupied.tar.gz");
    fs::write(&occupied_archive, "owned by another export\n").expect("write occupied archive");
    let occupied = export_development_context(DevelopmentContextExportRequest {
        project_id: "project-occupied".to_string(),
        repositories: vec![DevelopmentRepositorySelection {
            workspace_id: "workspace".to_string(),
            worktree_path: repository.clone(),
            role: DevelopmentRepositoryRole::Primary,
        }],
        archive_path: occupied_archive.clone(),
    })
    .expect_err("occupied archive should fail without replacement");
    assert!(occupied.to_string().contains("already exists"));
    assert_eq!(
        fs::read_to_string(occupied_archive).expect("read occupied archive"),
        "owned by another export\n"
    );

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("tracked.txt", repository.join("dirty-link"))
            .expect("create dirty symlink");
        let dirty = one_repo_export(&root, &repository, "dirty-link")
            .expect_err("dirty symlink should fail");
        assert!(dirty.to_string().contains("must be a regular file"));
        fs::remove_file(repository.join("dirty-link")).expect("remove dirty symlink");
    }

    fs::write(
        repository.join("dirty-lfs.bin"),
        "version https://git-lfs.github.com/spec/v1\noid sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nsize 1\n",
    )
    .expect("write dirty LFS pointer");
    let dirty_lfs = one_repo_export(&root, &repository, "dirty-lfs")
        .expect_err("dirty LFS pointer should fail");
    assert!(dirty_lfs
        .to_string()
        .contains("dirty overlay contains a Git LFS pointer"));

    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn rejects_in_repository_outputs_shallow_sources_and_oversized_index_blobs() {
    let root = test_root("bounded-sources");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");

    let inside = export_development_context(DevelopmentContextExportRequest {
        project_id: "project-inside-output".to_string(),
        repositories: vec![DevelopmentRepositorySelection {
            workspace_id: "workspace".to_string(),
            worktree_path: repository.clone(),
            role: DevelopmentRepositoryRole::Primary,
        }],
        archive_path: repository.join("generated/context.tar.gz"),
    })
    .expect_err("output inside source should fail");
    assert!(inside.to_string().contains("cannot be created inside"));
    assert!(!repository.join("generated").exists());

    fs::write(repository.join("second.txt"), "second\n").expect("write second commit");
    git(&repository, &["add", "second.txt"]);
    git(&repository, &["commit", "-m", "second"]);
    let shallow = root.join("shallow");
    let source_url = format!("file://{}", repository.display());
    git_in(
        &root,
        &[
            "clone",
            "--depth",
            "1",
            &source_url,
            shallow.to_str().expect("shallow path"),
        ],
    );
    let shallow_error =
        one_repo_export(&root, &shallow, "shallow").expect_err("shallow source should fail");
    assert!(shallow_error.to_string().contains("is shallow"));

    let oversized = vec![b'x'; MAX_OVERLAY_FILE_BYTES as usize + 1];
    fs::write(repository.join("oversized.bin"), oversized).expect("write oversized file");
    git(&repository, &["add", "oversized.bin"]);
    let oversized_error = one_repo_export(&root, &repository, "oversized-index")
        .expect_err("oversized staged blob should fail before reading it");
    assert!(oversized_error.to_string().contains("maximum is 16777216"));

    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn bounds_bundle_writes_and_handles_unicode_ignore_patterns() {
    let root = test_root("bounded-bundle-and-unicode-ignore");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");

    let bounded_bundle = root.join("bounded.bundle");
    let bundle_error = create_git_bundle(&repository, &bounded_bundle, 64)
        .expect_err("small bundle limit should stop the Git writer");
    assert!(bundle_error.to_string().contains("exceeds 64 bytes"));
    assert!(!bounded_bundle.exists());

    fs::write(repository.join(".charioxignore"), "**/*.pem\n")
        .expect("write context ignore patterns");
    let unicode_directory = repository.join("κλειδί");
    fs::create_dir_all(&unicode_directory).expect("create Unicode directory");
    fs::write(unicode_directory.join("秘密.pem"), "not transferred\n")
        .expect("write ignored Unicode path");
    fs::write(unicode_directory.join("included.txt"), "transferred\n")
        .expect("write included Unicode path");

    let exported = one_repo_export(&root, &repository, "unicode-ignore")
        .expect("Unicode ignore matching should not panic");
    let paths = exported.manifest.repositories[0]
        .overlay
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<BTreeSet<_>>();
    assert!(!paths.contains("κλειδί/秘密.pem"));
    assert!(paths.contains("κλειδί/included.txt"));

    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn bounds_origin_derived_target_directories_with_a_stable_suffix() {
    let root = test_root("bounded-target-directory");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");
    let long_name = "a".repeat(400);
    let origin = format!("https://example.com/org/{long_name}.git");
    git(&repository, &["remote", "add", "origin", &origin]);

    let first = one_repo_export(&root, &repository, "long-target-first")
        .expect("long origin should export with a bounded target");
    let target = &first.manifest.repositories[0].target_directory;
    assert!(target.len() <= MAX_TARGET_DIRECTORY_BASE_BYTES);
    assert!(target.starts_with(&"a".repeat(100)));
    let repeated = one_repo_export(&root, &repository, "long-target-repeated")
        .expect("repeated long origin should keep the same target");
    assert_eq!(repeated.manifest.repositories[0].target_directory, *target);

    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn fails_closed_on_invalid_ignore_files_and_bounded_path_enumeration() {
    let root = test_root("bounded-enumeration-and-ignore");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");

    fs::write(repository.join(".charioxignore"), [0xff, 0xfe]).expect("write invalid ignore file");
    let invalid = one_repo_export(&root, &repository, "invalid-ignore")
        .expect_err("invalid UTF-8 ignore file should fail closed");
    assert!(invalid.to_string().contains("valid UTF-8"));
    assert!(!root.join("invalid-ignore.tar.gz").exists());
    assert_no_export_temporaries(&root);

    fs::write(
        repository.join(".charioxignore"),
        vec![b'a'; MAX_CONTEXT_IGNORE_BYTES as usize + 1],
    )
    .expect("write oversized ignore file");
    let oversized = one_repo_export(&root, &repository, "oversized-ignore")
        .expect_err("oversized ignore file should fail before reading it");
    assert!(oversized.to_string().contains("maximum is 262144"));
    assert!(!root.join("oversized-ignore.tar.gz").exists());
    assert_no_export_temporaries(&root);

    fs::remove_file(repository.join(".charioxignore")).expect("remove ignore file");
    fs::write(repository.join("one.txt"), "one\n").expect("write first untracked file");
    fs::write(repository.join("two.txt"), "two\n").expect("write second untracked file");
    let enumeration = stream_git_nul_records(
        &repository,
        &["ls-files", "--others", "--exclude-standard", "-z", "--"],
        1,
        |_| Ok(()),
    )
    .expect_err("path enumeration should stop at its record limit");
    assert!(enumeration
        .to_string()
        .contains("returned more than 1 records"));

    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn publishes_archives_without_clobbering_an_existing_destination() {
    let root = test_root("atomic-archive-publication");
    let temporary = root.join("temporary.tar.gz");
    let destination = root.join("destination.tar.gz");
    fs::write(&temporary, "complete archive\n").expect("write temporary archive");
    fs::write(&destination, "existing archive\n").expect("write existing archive");

    let error = publish_archive_no_clobber(&temporary, &destination, &root)
        .expect_err("publication must not replace an occupied destination");
    assert!(error.to_string().contains("already exists"));
    assert_eq!(
        fs::read_to_string(destination).expect("read existing archive"),
        "existing archive\n"
    );

    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn bounds_manifest_metadata_and_git_text_before_retaining_them() {
    let mut budget = ManifestMemoryBudget::new();
    budget
        .consume(MAX_MANIFEST_BYTES)
        .expect("exact manifest budget should fit");
    let budget_error = budget.consume(1).expect_err("manifest budget must be hard");
    assert!(budget_error
        .to_string()
        .contains("manifest metadata exceeds"));

    let root = test_root("bounded-git-text");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");
    let oversized_origin = format!("https://example.com/{}.git", "a".repeat(MAX_GIT_TEXT_BYTES));
    git(&repository, &["remote", "add", "origin", &oversized_origin]);
    let error = one_repo_export(&root, &repository, "oversized-origin")
        .expect_err("oversized Git metadata should fail before retention");
    assert!(error
        .to_string()
        .contains("Git command output exceeds 65536 bytes"));
    assert_no_export_temporaries(&root);

    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn concurrent_exports_reserve_distinct_staging_and_archive_paths() {
    let root = test_root("concurrent-exports");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");
    let first_root = root.clone();
    let first_repository = repository.clone();
    let first = std::thread::spawn(move || {
        one_repo_export(&first_root, &first_repository, "concurrent-first")
    });
    let second_root = root.clone();
    let second_repository = repository.clone();
    let second = std::thread::spawn(move || {
        one_repo_export(&second_root, &second_repository, "concurrent-second")
    });

    first
        .join()
        .expect("first export thread should not panic")
        .expect("first concurrent export should succeed");
    second
        .join()
        .expect("second export thread should not panic")
        .expect("second concurrent export should succeed");
    assert_no_export_temporaries(&root);

    fs::remove_dir_all(root).expect("remove test root");
}

fn one_repo_export(
    root: &Path,
    repository: &Path,
    label: &str,
) -> Result<DevelopmentContextExportResult, DaemonError> {
    export_development_context(DevelopmentContextExportRequest {
        project_id: format!("project-{label}"),
        repositories: vec![DevelopmentRepositorySelection {
            workspace_id: "workspace".to_string(),
            worktree_path: repository.to_path_buf(),
            role: DevelopmentRepositoryRole::Primary,
        }],
        archive_path: root.join(format!("{label}.tar.gz")),
    })
}

fn assert_file_state_bytes(unpacked: &Path, state: &DevelopmentFileState, expected: &[u8]) {
    let DevelopmentFileState::File { object_path, .. } = state else {
        panic!("expected file state, got {state:?}");
    };
    assert_eq!(
        fs::read(unpacked.join(object_path)).expect("read overlay object"),
        expected
    );
}

fn unpack_archive(archive: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create unpack destination");
    let file = File::open(archive).expect("open archive");
    tar::Archive::new(GzDecoder::new(file))
        .unpack(destination)
        .expect("unpack archive");
}

fn init_repository(path: &Path, file: &str, contents: &str) {
    fs::create_dir_all(path).expect("create repository");
    git(path, &["init", "-b", "main"]);
    git(path, &["config", "user.email", "tests@chariox.local"]);
    git(path, &["config", "user.name", "Chariox Tests"]);
    fs::write(path.join(file), contents).expect("write repository file");
    git(path, &["add", file]);
    git(path, &["commit", "-m", "initial"]);
}

fn git(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_in(path: &Path, args: &[&str]) {
    git(path, args);
}

fn git_text_test(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .expect("run git");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("git text")
        .trim()
        .to_string()
}

fn test_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "chariox-managed-context-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create test root");
    path
}

fn assert_no_export_temporaries(root: &Path) {
    let temporary = fs::read_dir(root)
        .expect("read test root")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .find(|name| name.starts_with(".tmp-chariox-managed-context"));
    assert_eq!(temporary, None);
}
