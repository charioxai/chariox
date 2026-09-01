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
                worktree_id: None,
                worktree_path: primary.clone(),
                role: DevelopmentRepositoryRole::Primary,
            },
            DevelopmentRepositorySelection {
                workspace_id: supporting.display().to_string(),
                worktree_id: None,
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
                worktree_id: None,
                worktree_path: primary.clone(),
                role: DevelopmentRepositoryRole::Primary,
            },
            DevelopmentRepositorySelection {
                workspace_id: supporting.display().to_string(),
                worktree_id: None,
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
                worktree_id: None,
                worktree_path: repository.clone(),
                role: DevelopmentRepositoryRole::Primary,
            },
            DevelopmentRepositorySelection {
                workspace_id: "two".to_string(),
                worktree_id: None,
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
            worktree_id: None,
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
            worktree_id: None,
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
            worktree_id: None,
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

#[test]
fn imports_two_repositories_with_exact_index_and_worktree_states() {
    let root = test_root("import-two-repositories");
    let primary = root.join("source-primary");
    let supporting = root.join("source-supporting");
    init_repository(&primary, "modified.txt", "base\n");
    fs::write(primary.join("deleted.txt"), "delete me\n").expect("write deleted fixture");
    git(&primary, &["add", "deleted.txt"]);
    git(&primary, &["commit", "-m", "add deleted fixture"]);
    init_repository(&supporting, "supporting.txt", "support\n");
    git(
        &primary,
        &[
            "remote",
            "add",
            "origin",
            "https://user:secret@example.com/org/primary.git",
        ],
    );
    git(
        &primary,
        &["update-ref", "refs/remotes/origin/main", "HEAD"],
    );
    git(
        &primary,
        &["branch", "--set-upstream-to", "origin/main", "main"],
    );
    fs::write(primary.join("modified.txt"), "staged\n").expect("write staged state");
    git(&primary, &["add", "modified.txt"]);
    fs::write(primary.join("modified.txt"), "working\n").expect("write working state");
    fs::remove_file(primary.join("deleted.txt")).expect("remove tracked fixture");
    git(&primary, &["add", "deleted.txt"]);
    fs::write(primary.join("untracked.txt"), "untracked\n").expect("write untracked state");

    let archive_path = root.join("context.tar.gz");
    let exported = export_development_context(DevelopmentContextExportRequest {
        project_id: "project-import".to_string(),
        repositories: vec![
            DevelopmentRepositorySelection {
                workspace_id: "primary-workspace".to_string(),
                worktree_id: None,
                worktree_path: primary,
                role: DevelopmentRepositoryRole::Primary,
            },
            DevelopmentRepositorySelection {
                workspace_id: "supporting-workspace".to_string(),
                worktree_id: None,
                worktree_path: supporting,
                role: DevelopmentRepositoryRole::Supporting,
            },
        ],
        archive_path: archive_path.clone(),
    })
    .expect("export import fixture");
    let destination = root.join("managed/project-import");
    let imported = import_development_context(DevelopmentContextImportRequest {
        archive_path,
        expected_archive_sha256: exported.archive_sha256,
        expected_project_id: "project-import".to_string(),
        expected_source_repositories: None,
        destination_root: destination.clone(),
    })
    .expect("import development context");

    assert_eq!(
        imported.destination_root,
        fs::canonicalize(destination.parent().expect("destination parent"))
            .expect("canonical destination parent")
            .join("project-import")
    );
    assert_eq!(imported.repositories.len(), 2);
    let primary_import = imported
        .repositories
        .iter()
        .find(|repository| repository.role == DevelopmentRepositoryRole::Primary)
        .expect("primary import mapping");
    assert_eq!(primary_import.repository_id, imported.primary_repository_id);
    assert!(imported
        .repositories
        .iter()
        .any(|repository| repository.role == DevelopmentRepositoryRole::Supporting));
    assert_eq!(
        fs::read_to_string(primary_import.destination_path.join("modified.txt"))
            .expect("read imported working state"),
        "working\n"
    );
    assert_eq!(
        git_text_test(&primary_import.destination_path, &["show", ":modified.txt"]),
        "staged"
    );
    assert_eq!(
        fs::read_to_string(primary_import.destination_path.join("untracked.txt"))
            .expect("read imported untracked file"),
        "untracked\n"
    );
    assert!(!primary_import.destination_path.join("deleted.txt").exists());
    assert_eq!(
        git_text_test(
            &primary_import.destination_path,
            &["remote", "get-url", "origin"]
        ),
        "https://example.com/org/primary.git"
    );
    assert_eq!(
        git_text_test(
            &primary_import.destination_path,
            &["config", "--local", "--get", "branch.main.remote"]
        ),
        "origin"
    );
    assert_eq!(
        git_text_test(
            &primary_import.destination_path,
            &["config", "--local", "--get", "branch.main.merge"]
        ),
        "refs/heads/main"
    );
    assert!(!git_status_test(
        &primary_import.destination_path,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            "refs/remotes/origin/main"
        ]
    ));
    assert_eq!(
        git_text_test(&primary_import.destination_path, &["rev-parse", "HEAD"]),
        primary_import.head_sha
    );
    assert_no_import_temporaries(&root.join("managed"));

    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn import_rejects_wrong_bindings_corruption_and_occupied_destinations_atomically() {
    let root = test_root("import-rejections");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");
    fs::write(repository.join("dirty.txt"), "dirty\n").expect("write dirty file");
    let exported = one_repo_export(&root, &repository, "import-source")
        .expect("export import rejection fixture");

    let wrong_project_destination = root.join("managed/wrong-project");
    let wrong_project = import_development_context(DevelopmentContextImportRequest {
        archive_path: exported.archive_path.clone(),
        expected_archive_sha256: exported.archive_sha256.clone(),
        expected_project_id: "different-project".to_string(),
        expected_source_repositories: None,
        destination_root: wrong_project_destination.clone(),
    })
    .expect_err("wrong project binding should fail");
    assert!(wrong_project
        .to_string()
        .contains("project id does not match"));
    assert!(!wrong_project_destination.exists());

    let corrupt_archive = root.join("corrupt.tar.gz");
    let mut corrupt_bytes = fs::read(&exported.archive_path).expect("read source archive");
    let middle = corrupt_bytes.len() / 2;
    corrupt_bytes[middle] ^= 0xff;
    fs::write(&corrupt_archive, corrupt_bytes).expect("write corrupt archive");
    let corrupt_destination = root.join("managed/corrupt");
    let corrupt = import_development_context(DevelopmentContextImportRequest {
        archive_path: corrupt_archive.clone(),
        expected_archive_sha256: sha256_file(&corrupt_archive).expect("hash corrupt archive"),
        expected_project_id: exported.manifest.project_id.clone(),
        expected_source_repositories: None,
        destination_root: corrupt_destination.clone(),
    })
    .expect_err("corrupt archive should fail before publication");
    match &corrupt {
        DaemonError::ManagedContext {
            code,
            operation,
            retryable,
            ..
        } => assert!(
            (*code == "invalid_managed_context"
                && *operation == "managed development context"
                && !retryable)
                || (*code == "managed_context_unavailable"
                    && (operation.starts_with("read development context")
                        || *operation == "finish development context archive")
                    && *retryable),
            "unexpected corruption error: {corrupt}"
        ),
        _ => panic!("unexpected corruption error: {corrupt}"),
    }
    assert!(!corrupt_destination.exists());

    let occupied = root.join("managed/occupied");
    fs::create_dir_all(&occupied).expect("create occupied destination");
    fs::write(occupied.join("owner.txt"), "not the importer\n").expect("write occupied marker");
    let occupied_error = import_development_context(DevelopmentContextImportRequest {
        archive_path: exported.archive_path,
        expected_archive_sha256: exported.archive_sha256,
        expected_project_id: exported.manifest.project_id,
        expected_source_repositories: None,
        destination_root: occupied.clone(),
    })
    .expect_err("occupied destination should not be replaced");
    assert!(occupied_error.to_string().contains("already exists"));
    assert_eq!(
        fs::read_to_string(occupied.join("owner.txt")).expect("read occupied marker"),
        "not the importer\n"
    );
    assert_no_import_temporaries(&root.join("managed"));

    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn supporting_repository_failure_publishes_no_partial_project() {
    let root = test_root("import-supporting-failure");
    let primary = root.join("primary");
    let supporting = root.join("supporting");
    init_repository(&primary, "primary.txt", "primary\n");
    init_repository(&supporting, "supporting.txt", "supporting\n");
    let source_archive = root.join("source.tar.gz");
    let exported = export_development_context(DevelopmentContextExportRequest {
        project_id: "project-supporting-failure".to_string(),
        repositories: vec![
            DevelopmentRepositorySelection {
                workspace_id: "primary".to_string(),
                worktree_id: None,
                worktree_path: primary,
                role: DevelopmentRepositoryRole::Primary,
            },
            DevelopmentRepositorySelection {
                workspace_id: "supporting".to_string(),
                worktree_id: None,
                worktree_path: supporting,
                role: DevelopmentRepositoryRole::Supporting,
            },
        ],
        archive_path: source_archive,
    })
    .expect("export two-repository fixture");
    let unpacked = root.join("unpacked-corrupt-supporting");
    unpack_archive(&exported.archive_path, &unpacked);
    fs::remove_file(unpacked.join("manifest.json")).expect("remove extracted manifest");
    let mut broken_manifest = exported.manifest;
    let supporting_manifest = broken_manifest
        .repositories
        .iter_mut()
        .find(|repository| repository.role == DevelopmentRepositoryRole::Supporting)
        .expect("supporting manifest");
    let supporting_bundle = unpacked.join(&supporting_manifest.bundle_path);
    fs::write(&supporting_bundle, "not a Git bundle\n").expect("corrupt supporting bundle");
    supporting_manifest.bundle_size_bytes = fs::metadata(&supporting_bundle)
        .expect("supporting bundle metadata")
        .len();
    supporting_manifest.bundle_sha256 =
        sha256_file(&supporting_bundle).expect("hash corrupt supporting bundle");
    let broken_archive = root.join("broken-supporting.tar.gz");
    let broken_file = private_create_new(&broken_archive).expect("reserve broken archive");
    write_archive(&broken_archive, broken_file, &unpacked, &broken_manifest)
        .expect("package internally consistent broken archive");

    let destination = root.join("managed/broken-supporting");
    let error = import_development_context(DevelopmentContextImportRequest {
        archive_path: broken_archive.clone(),
        expected_archive_sha256: sha256_file(&broken_archive).expect("hash broken archive"),
        expected_project_id: broken_manifest.project_id,
        expected_source_repositories: None,
        destination_root: destination.clone(),
    })
    .expect_err("invalid supporting bundle should abort the whole import");
    assert!(
        error.to_string().contains("bundle") || error.to_string().contains("clone"),
        "unexpected import error: {error}"
    );
    assert!(!destination.exists());
    assert_no_import_temporaries(&root.join("managed"));

    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn import_rejects_unsafe_manifest_paths_and_archive_symlinks() {
    let root = test_root("import-unsafe-manifest");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");
    let exported = one_repo_export(&root, &repository, "unsafe-import-source")
        .expect("export unsafe import fixture");
    let unpacked = root.join("unpacked-unsafe-manifest");
    unpack_archive(&exported.archive_path, &unpacked);
    fs::remove_file(unpacked.join("manifest.json")).expect("remove extracted manifest");
    let mut unsafe_manifest = exported.manifest.clone();
    unsafe_manifest.repositories[0].target_directory = "../escape".to_string();
    let unsafe_archive = root.join("unsafe-manifest.tar.gz");
    let unsafe_file = private_create_new(&unsafe_archive).expect("reserve unsafe archive");
    write_archive(&unsafe_archive, unsafe_file, &unpacked, &unsafe_manifest)
        .expect("package unsafe manifest fixture");
    let unsafe_destination = root.join("managed/unsafe-manifest");
    let unsafe_error = import_development_context(DevelopmentContextImportRequest {
        archive_path: unsafe_archive.clone(),
        expected_archive_sha256: sha256_file(&unsafe_archive).expect("hash unsafe archive"),
        expected_project_id: unsafe_manifest.project_id,
        expected_source_repositories: None,
        destination_root: unsafe_destination.clone(),
    })
    .expect_err("unsafe target directory should fail before extraction");
    assert!(unsafe_error
        .to_string()
        .contains("target directory is invalid"));
    assert!(!unsafe_destination.exists());

    #[cfg(unix)]
    {
        let archive_symlink = root.join("archive-symlink.tar.gz");
        std::os::unix::fs::symlink(&exported.archive_path, &archive_symlink)
            .expect("create archive symlink");
        let symlink_destination = root.join("managed/archive-symlink");
        let symlink_error = import_development_context(DevelopmentContextImportRequest {
            archive_path: archive_symlink,
            expected_archive_sha256: exported.archive_sha256,
            expected_project_id: exported.manifest.project_id,
            expected_source_repositories: None,
            destination_root: symlink_destination.clone(),
        })
        .expect_err("archive symlink should fail closed");
        assert!(symlink_error.to_string().contains("cannot be a symlink"));
        assert!(!symlink_destination.exists());
    }
    assert_no_import_temporaries(&root.join("managed"));

    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn import_rejects_git_administrative_overlay_paths() {
    let root = test_root("import-git-admin-overlay");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");
    fs::write(repository.join("dirty.txt"), "malicious\n").expect("write dirty fixture");
    let exported =
        one_repo_export(&root, &repository, "git-admin-overlay").expect("export overlay fixture");
    let unpacked = root.join("unpacked");
    unpack_archive(&exported.archive_path, &unpacked);
    fs::remove_file(unpacked.join("manifest.json")).expect("remove extracted manifest");
    for (index, alias) in [
        ".GIT/config",
        ".git./config",
        "git~1/config",
        ".\u{200c}git/config",
        "git~1:$INDEX_ALLOCATION/config",
    ]
    .into_iter()
    .enumerate()
    {
        let mut manifest = exported.manifest.clone();
        manifest.repositories[0].overlay[0].path = alias.to_string();
        let archive = root.join(format!("malicious-git-admin-overlay-{index}.tar.gz"));
        let file = private_create_new(&archive).expect("reserve malicious archive");
        write_archive(&archive, file, &unpacked, &manifest).expect("package malicious archive");
        let destination = root.join(format!("managed/git-admin-overlay-{index}"));
        let error = import_development_context(DevelopmentContextImportRequest {
            archive_path: archive.clone(),
            expected_archive_sha256: sha256_file(&archive).expect("hash malicious archive"),
            expected_project_id: manifest.project_id,
            expected_source_repositories: None,
            destination_root: destination.clone(),
        })
        .expect_err("Git administrative overlay must fail closed");
        assert!(
            error.to_string().contains("unsafe repository path"),
            "unexpected alias rejection for {alias}: {error}"
        );
        assert!(!destination.exists());
    }
    assert_no_import_temporaries(&root.join("managed"));
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn export_fails_on_nonportable_git_admin_aliases() {
    let root = test_root("export-git-admin-alias");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");
    fs::create_dir(repository.join("git~1")).expect("create NTFS alias fixture");
    fs::write(repository.join("git~1/config"), "must not be omitted\n")
        .expect("write NTFS alias fixture");
    let error = one_repo_export(&root, &repository, "git-admin-alias")
        .expect_err("nonportable dirty Git alias must fail export");
    assert!(error.to_string().contains("unsafe repository path"));
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn export_rejects_checkout_expanding_attributes() {
    let root = test_root("checkout-attributes");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "$Id$\n");
    fs::write(
        repository.join(".gitattributes"),
        "tracked.txt ident working-tree-encoding=UTF-32\n",
    )
    .expect("write transforming attributes");
    git(&repository, &["add", ".gitattributes"]);
    git(&repository, &["commit", "-m", "add checkout transforms"]);
    let error = one_repo_export(&root, &repository, "checkout-attributes")
        .expect_err("checkout transforms must fail export");
    assert!(error
        .to_string()
        .contains("checkout-transforming attribute"));
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn archive_snapshot_is_immutable_after_source_changes() {
    let root = test_root("archive-snapshot");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");
    let exported =
        one_repo_export(&root, &repository, "snapshot-source").expect("export snapshot fixture");
    let original = fs::read(&exported.archive_path).expect("read source archive");
    let snapshot_path = root.join("private-snapshot.tar.gz");
    let (snapshot, size, digest) =
        super::import::snapshot_and_hash_archive(&exported.archive_path, &snapshot_path)
            .expect("snapshot archive");
    fs::write(&exported.archive_path, vec![0_u8; original.len()])
        .expect("mutate source archive in place");
    assert_eq!(size, original.len() as u64);
    assert_eq!(digest, sha256_bytes(&original));
    assert_eq!(fs::read(&snapshot_path).expect("read snapshot"), original);
    let artifacts = root.join("artifacts");
    create_private_directory(&artifacts).expect("create artifacts root");
    let manifest =
        extract_and_verify_archive(snapshot, &exported.manifest.project_id, None, &artifacts)
            .expect("parse immutable snapshot");
    assert_eq!(manifest, exported.manifest);
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn import_rejects_pax_extension_bomb_before_body_allocation() {
    let root = test_root("pax-bomb");
    let archive = root.join("pax-bomb.tar.gz");
    let file = private_create_new(&archive).expect("reserve PAX archive");
    let mut encoder = GzBuilder::new()
        .mtime(0)
        .write(file, Compression::default());
    let header = raw_tar_header("pax", b'x', 32 * 1024 * 1024);
    encoder.write_all(&header).expect("write PAX header");
    let zeros = [0_u8; 64 * 1024];
    for _ in 0..512 {
        encoder
            .write_all(&zeros)
            .expect("write compressible PAX body");
    }
    encoder.finish().expect("finish PAX archive");
    let destination = root.join("managed/pax-bomb");
    let error = import_development_context(DevelopmentContextImportRequest {
        archive_path: archive.clone(),
        expected_archive_sha256: sha256_file(&archive).expect("hash PAX archive"),
        expected_project_id: "pax-bomb".to_string(),
        expected_source_repositories: None,
        destination_root: destination.clone(),
    })
    .expect_err("PAX extension metadata must fail closed");
    assert!(error.to_string().contains("unsupported tar extension"));
    assert!(!destination.exists());
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn materialization_rejects_large_current_tree_before_checkout() {
    let root = test_root("checkout-budget");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");
    fs::write(repository.join("compressible.bin"), vec![0_u8; 1024 * 1024])
        .expect("write compressible blob");
    git(&repository, &["add", "compressible.bin"]);
    git(&repository, &["commit", "-m", "large compressed blob"]);
    let exported =
        one_repo_export(&root, &repository, "checkout-budget").expect("export large tree");
    let unpacked = root.join("unpacked");
    unpack_archive(&exported.archive_path, &unpacked);
    let destination = root.join("materialized");
    let error = prepare_repository(
        &exported.manifest.repositories[0],
        &unpacked,
        &destination,
        512 * 1024,
        MAX_MATERIALIZED_ENTRIES_PER_REPOSITORY,
    )
    .expect_err("checkout budget must reject the current tree");
    assert!(error.to_string().contains("checkout budget"));
    assert!(!destination.join("compressible.bin").exists());
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn multi_repository_entry_budget_rejects_before_any_checkout() {
    let root = test_root("project-entry-budget");
    let primary = root.join("primary");
    let supporting = root.join("supporting");
    init_repository(&primary, "primary.txt", "");
    init_repository(&supporting, "supporting.txt", "");
    let archive = root.join("project-entry-budget.tar.gz");
    let exported = export_development_context(DevelopmentContextExportRequest {
        project_id: "project-entry-budget".to_string(),
        repositories: vec![
            DevelopmentRepositorySelection {
                workspace_id: "primary".to_string(),
                worktree_id: None,
                worktree_path: primary,
                role: DevelopmentRepositoryRole::Primary,
            },
            DevelopmentRepositorySelection {
                workspace_id: "supporting".to_string(),
                worktree_id: None,
                worktree_path: supporting,
                role: DevelopmentRepositoryRole::Supporting,
            },
        ],
        archive_path: archive.clone(),
    })
    .expect("export entry-budget fixture");
    let destination = root.join("managed/project-entry-budget");
    let error = super::import::import_development_context_with_budgets(
        DevelopmentContextImportRequest {
            archive_path: archive,
            expected_archive_sha256: exported.archive_sha256,
            expected_project_id: exported.manifest.project_id,
            expected_source_repositories: None,
            destination_root: destination.clone(),
        },
        MAX_CHECKOUT_BYTES_PER_PROJECT,
        3,
    )
    .expect_err("project-wide entry budget must reject two repositories");
    assert!(error.to_string().contains("entry materialization budget"));
    assert!(!destination.exists());
    assert_no_import_temporaries(&root.join("managed"));
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn overlay_entries_count_toward_prepare_budget() {
    let root = test_root("overlay-entry-budget");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");
    fs::create_dir(repository.join("nested")).expect("create nested overlay directory");
    fs::write(repository.join("nested/dirty.txt"), "dirty\n").expect("write untracked overlay");
    let exported = one_repo_export(&root, &repository, "overlay-entry-budget")
        .expect("export overlay budget fixture");
    let destination = root.join("managed/overlay-entry-budget");
    let error = super::import::import_development_context_with_budgets(
        DevelopmentContextImportRequest {
            archive_path: exported.archive_path,
            expected_archive_sha256: exported.archive_sha256,
            expected_project_id: exported.manifest.project_id,
            expected_source_repositories: None,
            destination_root: destination.clone(),
        },
        MAX_CHECKOUT_BYTES_PER_PROJECT,
        3,
    )
    .expect_err("nested overlay must count toward the prepare budget");
    assert!(error.to_string().contains("repository overlay"));
    assert!(!destination.exists());
    assert_no_import_temporaries(&root.join("managed"));
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn empty_repository_root_cannot_exceed_zero_remaining_budget() {
    let root = test_root("empty-root-budget");
    let repository = root.join("repository");
    fs::create_dir(&repository).expect("create empty repository");
    git(&repository, &["init", "-b", "main"]);
    git(
        &repository,
        &["config", "user.email", "tests@chariox.local"],
    );
    git(&repository, &["config", "user.name", "Chariox Tests"]);
    git(&repository, &["commit", "--allow-empty", "-m", "empty"]);
    let exported =
        one_repo_export(&root, &repository, "empty-root-budget").expect("export empty repository");
    let destination = root.join("managed/empty-root-budget");
    let error = super::import::import_development_context_with_budgets(
        DevelopmentContextImportRequest {
            archive_path: exported.archive_path,
            expected_archive_sha256: exported.archive_sha256,
            expected_project_id: exported.manifest.project_id,
            expected_source_repositories: None,
            destination_root: destination.clone(),
        },
        0,
        0,
    )
    .expect_err("repository root must count against zero remaining budget");
    assert!(error.to_string().contains("repository root"));
    assert!(!destination.exists());
    assert_no_import_temporaries(&root.join("managed"));
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn isolated_git_commands_remove_hostile_repository_environment() {
    let root = test_root("isolated-git-environment");
    let repository = root.join("repository");
    let decoy = root.join("decoy");
    init_repository(&repository, "tracked.txt", "base\n");
    init_repository(&decoy, "decoy.txt", "decoy\n");
    let mut command = Command::new("git");
    command
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&repository)
        .env("GIT_DIR", decoy.join(".git"))
        .env("GIT_WORK_TREE", &decoy);
    super::git::configure_isolated_git_environment(&mut command);
    let output = command.output().expect("run isolated Git command");
    assert!(output.status.success());
    assert_eq!(
        PathBuf::from(
            String::from_utf8(output.stdout)
                .expect("Git path UTF-8")
                .trim()
        ),
        fs::canonicalize(&repository).expect("canonical repository")
    );
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn bundle_verification_rejects_oversized_header_lines() {
    let root = test_root("bundle-header-cap");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");
    let bundle = root.join("oversized.bundle");
    let mut bytes = b"# v2 git bundle\n".to_vec();
    bytes.extend(std::iter::repeat_n(b'a', MAX_GIT_BUNDLE_HEADER_BYTES + 1));
    fs::write(&bundle, bytes).expect("write oversized bundle header");
    let error = verify_git_bundle(
        &repository,
        &bundle,
        &git_text_test(&repository, &["rev-parse", "HEAD"]),
    )
    .expect_err("oversized bundle header must fail closed");
    assert!(error.to_string().contains("bundle header exceeds"));
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn published_import_receipt_recovers_an_import_after_process_restart() {
    let root = test_root("publication-recovery");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");
    let exported = one_repo_export(&root, &repository, "publication-recovery")
        .expect("export publication fixture");
    let request = DevelopmentContextImportRequest {
        archive_path: exported.archive_path,
        expected_archive_sha256: exported.archive_sha256,
        expected_project_id: "project-publication-recovery".to_string(),
        expected_source_repositories: None,
        destination_root: root.join("managed/project"),
    };
    let receipt = import_development_context_with_publication(
        request.clone(),
        "ctx_publication_recovery".to_string(),
    )
    .expect("publish development context with receipt");

    let recovered = recover_development_context_publication(&request, "ctx_publication_recovery")
        .expect("recover published context")
        .expect("published receipt");
    assert_eq!(recovered, receipt);
    assert_eq!(recovered.repositories.len(), 1);
    assert_eq!(
        git_text_test(
            &recovered.repositories[0].destination_path,
            &["rev-parse", "HEAD"]
        ),
        recovered.repositories[0].head_sha
    );
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn failed_transfer_cleanup_removes_only_its_receipted_publication() {
    let root = test_root("failed-publication-cleanup");
    let repository = root.join("repository");
    init_repository(&repository, "tracked.txt", "base\n");
    let exported = one_repo_export(&root, &repository, "failed-publication-cleanup")
        .expect("export cleanup fixture");
    let destination_root = root.join("managed/project");
    import_development_context_with_publication(
        DevelopmentContextImportRequest {
            archive_path: exported.archive_path,
            expected_archive_sha256: exported.archive_sha256,
            expected_project_id: "project-failed-publication-cleanup".to_string(),
            expected_source_repositories: None,
            destination_root: destination_root.clone(),
        },
        "ctx_failed_publication".to_string(),
    )
    .expect("publish cleanup fixture");

    let error = cleanup_development_context_publication(&destination_root, "ctx_other")
        .expect_err("different publication must not be removed");
    assert!(matches!(
        error,
        DaemonError::ManagedContext {
            code: "invalid_managed_context",
            retryable: false,
            ..
        }
    ));
    assert!(destination_root.exists());
    cleanup_development_context_publication(&destination_root, "ctx_failed_publication")
        .expect("remove exact failed publication");
    assert!(!destination_root.exists());
    fs::remove_dir_all(root).expect("remove test root");
}

#[test]
fn deterministic_import_validation_errors_are_not_retryable() {
    let error = import_development_context(DevelopmentContextImportRequest {
        archive_path: PathBuf::from("/unused/invalid-context.tar.gz"),
        expected_archive_sha256: "not-a-sha256".to_string(),
        expected_project_id: "project-1".to_string(),
        expected_source_repositories: None,
        destination_root: PathBuf::from("/unused/destination"),
    })
    .expect_err("invalid digest should fail before filesystem access");
    assert!(matches!(
        error,
        DaemonError::ManagedContext {
            code: "invalid_managed_context",
            retryable: false,
            ..
        }
    ));
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
            worktree_id: None,
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

fn raw_tar_header(path: &str, entry_type: u8, size: u64) -> [u8; 512] {
    let mut header = [0_u8; 512];
    header[..path.len()].copy_from_slice(path.as_bytes());
    header[100..108].copy_from_slice(b"0000600\0");
    header[108..116].copy_from_slice(b"0000000\0");
    header[116..124].copy_from_slice(b"0000000\0");
    header[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    header[136..148].copy_from_slice(b"00000000000\0");
    header[148..156].fill(b' ');
    header[156] = entry_type;
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let checksum = header.iter().map(|byte| *byte as u64).sum::<u64>();
    header[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
    header
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

fn git_status_test(path: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(path)
        .status()
        .expect("run git status command")
        .success()
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

fn assert_no_import_temporaries(root: &Path) {
    let temporary = fs::read_dir(root)
        .expect("read import parent")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .find(|name| name.starts_with(".tmp-chariox-context-import"));
    assert_eq!(temporary, None);
}

#[cfg(unix)]
#[test]
fn publication_cleanup_rejects_a_replaced_symlink_ancestor() {
    use std::os::unix::fs::symlink;

    let root = test_root("cleanup-symlink-ancestor");
    let bound_parent = root.join("bound-parent");
    let moved_parent = root.join("moved-parent");
    let outside_parent = root.join("outside-parent");
    fs::create_dir_all(&bound_parent).expect("create bound parent");
    fs::create_dir_all(&outside_parent).expect("create outside parent");
    let canonical_parent = fs::canonicalize(&bound_parent).expect("canonical bound parent");
    let destination_root = canonical_parent.join("destination");
    let publication_id = "publication-1";
    let outside_staging = outside_parent.join(format!(
        ".tmp-chariox-context-import-{publication_id}.staging"
    ));
    fs::create_dir(&outside_staging).expect("create outside staging canary");
    fs::rename(&bound_parent, &moved_parent).expect("move bound parent");
    symlink(&outside_parent, &bound_parent).expect("replace parent with symlink");

    let error = cleanup_development_context_publication_staging(&destination_root, publication_id)
        .expect_err("changed ancestor binding must reject cleanup");
    assert!(matches!(
        error,
        DaemonError::ManagedContext {
            code: "invalid_managed_context",
            retryable: false,
            ..
        }
    ));
    assert!(outside_staging.exists());
    fs::remove_file(&bound_parent).expect("remove parent symlink");
    fs::remove_dir_all(root).expect("remove test root");
}
