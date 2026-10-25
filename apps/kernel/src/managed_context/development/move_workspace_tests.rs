//! MP-08 / MP-10 / MP-11: Move uses the ordinary Project snapshot contract.
use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn mk02_refuses_a_project_changed_during_export() {
    for (directory, change_current) in [(false, false), (true, false), (false, true)] {
        let root = test_root("move-changing-project");
        let _cleanup = PlainWorkspaceCleanup(root.clone());
        let primary = root.join("primary");
        let supporting = root.join("supporting");
        if directory {
            fs::create_dir(&primary).unwrap();
            fs::write(primary.join("input"), "before\n").unwrap();
        } else {
            init_repository(&primary, "input", "before\n");
        }
        init_repository(&supporting, "input", "support\n");
        let trigger = if change_current {
            &primary
        } else {
            &supporting
        };
        let _hook = change_during_bundle(&root, trigger, &primary.join("input"));
        let archive = root.join("snapshot.tar.gz");
        let result = export_development_context(DevelopmentContextExportRequest {
            project_id: "move-project".into(),
            repositories: vec![selection(&primary, true), selection(&supporting, false)],
            archive_path: archive.clone(),
        });
        assert_eq!(fs::read(primary.join("input")).unwrap(), b"after!\n");
        assert!(
            result.is_err(),
            "a changing Project must not publish a mixed snapshot"
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("changed while exporting"));
        assert!(!archive.exists());
        assert_no_export_temporaries(&root);
        assert_eq!(fs::read(supporting.join("input")).unwrap(), b"support\n");
    }
}

fn selection(path: &Path, primary: bool) -> DevelopmentRepositorySelection {
    DevelopmentRepositorySelection {
        workspace_id: path.to_string_lossy().into_owned(),
        worktree_id: None,
        worktree_path: path.into(),
        role: if primary {
            DevelopmentRepositoryRole::Primary
        } else {
            DevelopmentRepositoryRole::Supporting
        },
    }
}

// A real Git subprocess changes a source at a deterministic point inside the
// public export path. No timing race or production-only test hook is needed.
fn change_during_bundle(
    root: &Path,
    trigger: &Path,
    changed: &Path,
) -> Vec<TestEnvironmentVariableGuard> {
    let original = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    assert!(original.status.success());
    let original = String::from_utf8(original.stdout).unwrap();
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let wrapper = bin.join("git");
    fs::write(&wrapper, "#!/bin/sh\nif [ \"$1\" = bundle ] && [ \"$2\" = create ] && [ \"$PWD\" = \"$MK02_TRIGGER\" ]; then\n  printf 'after!\\n' > \"$MK02_CHANGED\"\nfi\nexec \"$MK02_GIT\" \"$@\"\n").unwrap();
    fs::set_permissions(wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let mut path = vec![bin];
    path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    vec![
        TestEnvironmentVariableGuard::set("MK02_GIT", Some(OsStr::new(original.trim()))),
        TestEnvironmentVariableGuard::set("MK02_TRIGGER", Some(trigger.as_os_str())),
        TestEnvironmentVariableGuard::set("MK02_CHANGED", Some(changed.as_os_str())),
        TestEnvironmentVariableGuard::set("PATH", Some(&std::env::join_paths(path).unwrap())),
    ]
}

#[test]
fn mk02_linked_worktree_binary_index_modes_directory_and_fresh_return() {
    let root = test_root("move-linked-return");
    let _cleanup = PlainWorkspaceCleanup(root.clone());
    let main = root.join("main");
    init_repository(&main, "input", "base\n");
    fs::write(main.join("removed"), "remove me\n").unwrap();
    git(&main, &["add", "removed"]);
    git(&main, &["commit", "-m", "second"]);
    let linked = root.join("linked");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-b",
            "move-branch",
            linked.to_str().unwrap(),
        ],
    );
    assert!(linked.join(".git").is_file());
    fs::write(linked.join("input"), b"index\0\xff\n").unwrap();
    fs::set_permissions(linked.join("input"), fs::Permissions::from_mode(0o755)).unwrap();
    git(&linked, &["add", "input"]);
    fs::write(linked.join("input"), b"working\0\xfe\n").unwrap();
    fs::set_permissions(linked.join("input"), fs::Permissions::from_mode(0o644)).unwrap();
    fs::remove_file(linked.join("removed")).unwrap();
    git(&linked, &["add", "removed"]);
    fs::write(linked.join("untracked"), b"new\0\x80\n").unwrap();
    let directory = root.join("notes");
    fs::create_dir_all(directory.join("nested/empty")).unwrap();
    fs::write(directory.join("nested/document"), b"office\0\x81\n").unwrap();
    let before = git_state(&linked);
    let exported = export_development_context(DevelopmentContextExportRequest {
        project_id: "move-project".into(),
        repositories: vec![selection(&linked, true), selection(&directory, false)],
        archive_path: root.join("outbound.tar.gz"),
    })
    .unwrap();
    let request = DevelopmentContextImportRequest {
        archive_path: exported.archive_path,
        expected_archive_sha256: exported.archive_sha256,
        expected_project_id: "move-project".into(),
        expected_source_repositories: None,
        destination_root: root.join("target"),
    };
    let imported = import_development_context(request.clone()).unwrap();
    let target = &imported.repositories[0].destination_path;
    let notes = &imported.repositories[1].destination_path;
    assert!(target.join(".git").is_dir());
    assert_eq!(
        fs::canonicalize(target.join(git_text_test(target, &["rev-parse", "--git-common-dir"])))
            .unwrap(),
        target.join(".git")
    );
    assert_eq!(git_state(target), before);
    assert_eq!(git_state(&linked), before);
    assert_eq!(
        fs::read(notes.join("nested/document")).unwrap(),
        b"office\0\x81\n"
    );
    assert!(notes.join("nested/empty").is_dir());
    assert!(!notes.join(".git").exists());

    // A return is a new snapshot; the source is never reset or merged.
    fs::write(target.join("input"), b"target edit\0\x82\n").unwrap();
    git(target, &["add", "input"]);
    fs::write(target.join("input"), b"return edit\0\x83\n").unwrap();
    fs::write(notes.join("nested/document"), "target document\n").unwrap();
    let divergent = git_state(target);
    assert!(import_development_context(request)
        .unwrap_err()
        .to_string()
        .contains("already exists"));
    assert_eq!(git_state(target), divergent);
    assert_eq!(git_state(&linked), before);
    let returned = export_development_context(DevelopmentContextExportRequest {
        project_id: "move-project".into(),
        repositories: vec![selection(target, true), selection(notes, false)],
        archive_path: root.join("return.tar.gz"),
    })
    .unwrap();
    let returned = import_development_context(DevelopmentContextImportRequest {
        archive_path: returned.archive_path,
        expected_archive_sha256: returned.archive_sha256,
        expected_project_id: "move-project".into(),
        expected_source_repositories: None,
        destination_root: root.join("fresh-home"),
    })
    .unwrap();
    assert_eq!(
        git_state(&returned.repositories[0].destination_path),
        divergent
    );
    assert_eq!(
        fs::read(
            returned.repositories[1]
                .destination_path
                .join("nested/document")
        )
        .unwrap(),
        b"target document\n"
    );
    assert_eq!(git_state(&linked), before);
}

fn git_state(root: &Path) -> Vec<Vec<u8>> {
    [
        vec!["rev-parse", "HEAD"],
        vec!["symbolic-ref", "--short", "HEAD"],
        vec!["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        vec!["diff", "--binary", "--cached"],
        vec!["diff", "--binary"],
        vec!["ls-files", "--stage", "-z"],
    ]
    .iter()
    .map(|args| git_bytes(root, args).unwrap())
    .collect()
}

#[test]
fn mk02_directory_uses_project_private_file_choices() {
    use crate::project_environment::*;
    let root = test_root("move-directory-selection");
    let _cleanup = PlainWorkspaceCleanup(root.clone());
    let directory = root.join("notes");
    fs::create_dir(&directory).unwrap();
    fs::write(
        directory.join(".gitignore"),
        "private.txt\nselected.txt\nprivate/\nunselected/\n",
    )
    .unwrap();
    fs::write(directory.join("private.txt"), "do not bring\n").unwrap();
    fs::write(directory.join("selected.txt"), "selected content\n").unwrap();
    fs::create_dir(directory.join("private")).unwrap();
    fs::create_dir_all(directory.join("unselected/empty")).unwrap();
    fs::write(directory.join("private/needed.bin"), b"selected\0bytes\n").unwrap();
    fs::write(directory.join("private/stays.txt"), "stay home\n").unwrap();
    let mut manifest = ProjectEnvironmentManifest {
        schema_version: 1,
        project_id: "move-project".into(),
        evidence_digest: "fixture".into(),
        entries: vec![],
        private_files: vec![],
        toolchain_hints: vec![],
        package_hints: vec![],
        service_hints: vec![],
    };
    let workspace = directory.to_string_lossy().into_owned();
    for path in ["selected.txt", "private/needed.bin"] {
        manifest.private_files.push(ProjectPrivateFileDecision {
            workspace_id: workspace.clone(),
            path: path.into(),
            bring: true,
            secret_looking: false,
            reason: "explicitly selected".into(),
        });
    }
    register_project_file_rules(&manifest, &BTreeMap::from([(workspace, directory.clone())]));
    let result = export_development_context(DevelopmentContextExportRequest {
        project_id: "move-project".into(),
        repositories: vec![selection(&directory, true)],
        archive_path: root.join("selected.tar.gz"),
    });
    forget_project_file_rules(&manifest);
    let exported = result.unwrap();
    let paths: Vec<_> = exported.manifest.repositories[0]
        .overlay
        .iter()
        .map(|e| e.path.as_str())
        .collect();
    assert!(paths.contains(&"selected.txt"));
    assert!(paths.contains(&"private/needed.bin"));
    assert!(!paths.contains(&"private/stays.txt"));
    assert!(!exported.manifest.repositories[0]
        .directories
        .iter()
        .any(|p| p.starts_with("unselected")));
    assert!(
        !paths.contains(&"private.txt"),
        "unselected ignored files must stay home"
    );
    let imported = import_development_context(DevelopmentContextImportRequest {
        archive_path: exported.archive_path,
        expected_archive_sha256: exported.archive_sha256,
        expected_project_id: "move-project".into(),
        expected_source_repositories: None,
        destination_root: root.join("selected-target"),
    })
    .unwrap();
    let restored = &imported.repositories[0].destination_path;
    assert_eq!(
        fs::read(restored.join("private/needed.bin")).unwrap(),
        b"selected\0bytes\n"
    );
    assert!(!restored.join("private/stays.txt").exists());
    assert!(!restored.join("unselected").exists());
}

#[test]
fn mk02_rejects_deleted_or_replaced_committed_symlinks() {
    for deletion in [false, true] {
        let root = test_root("move-dirty-head-symlink");
        let _cleanup = PlainWorkspaceCleanup(root.clone());
        let repository = root.join("repository");
        init_repository(&repository, "input", "base\n");
        std::os::unix::fs::symlink("input", repository.join("link")).unwrap();
        git(&repository, &["add", "link"]);
        git(&repository, &["commit", "-m", "internal symlink"]);
        fs::remove_file(repository.join("link")).unwrap();
        if !deletion {
            fs::write(repository.join("link"), "replacement\n").unwrap();
        }
        git(&repository, &["add", "link"]);
        let archive = root.join("snapshot.tar.gz");
        let result = export_development_context(DevelopmentContextExportRequest {
            project_id: "move-project".into(),
            repositories: vec![selection(&repository, true)],
            archive_path: archive.clone(),
        });
        assert!(
            result.is_err(),
            "dirty symlinks must be refused even after their index entry changes type"
        );
        assert!(result.unwrap_err().to_string().contains("dirty symlinks"));
        assert!(!archive.exists());
        assert_no_export_temporaries(&root);
    }
}

fn directory_glob_inclusion_roundtrips(pattern: &str) {
    let root = test_root("move-directory-glob-inclusion");
    let _cleanup = PlainWorkspaceCleanup(root.clone());
    let directory = root.join("notes");
    fs::create_dir_all(directory.join("private-extra/nested")).unwrap();
    fs::write(directory.join(".gitignore"), "private-extra/\n").unwrap();
    fs::write(directory.join(".charioxignore"), format!("!{pattern}\n")).unwrap();
    for path in [
        "private-extra/needed.txt",
        "private-extra/nested/needed.txt",
    ] {
        fs::write(directory.join(path), b"requested file\n").unwrap();
    }
    fs::write(directory.join("private-extra/stays.txt"), b"stay home\n").unwrap();
    let exported = export_development_context(DevelopmentContextExportRequest {
        project_id: "move-project".into(),
        repositories: vec![selection(&directory, true)],
        archive_path: root.join("selected.tar.gz"),
    })
    .unwrap();
    assert!(
        exported.manifest.repositories[0]
            .overlay
            .iter()
            .any(|entry| { entry.path == "private-extra/needed.txt" }),
        "glob inclusion must reach the requested file below an ignored directory"
    );
    let imported = import_development_context(DevelopmentContextImportRequest {
        archive_path: exported.archive_path,
        expected_archive_sha256: exported.archive_sha256,
        expected_project_id: "move-project".into(),
        expected_source_repositories: None,
        destination_root: root.join("selected-target"),
    })
    .unwrap();
    let restored = &imported.repositories[0].destination_path;
    assert_eq!(
        fs::read(restored.join("private-extra/needed.txt")).unwrap(),
        b"requested file\n"
    );
    assert!(!restored.join("private-extra/stays.txt").exists());
    if pattern.starts_with("**/") {
        assert_eq!(
            fs::read(restored.join("private-extra/nested/needed.txt")).unwrap(),
            b"requested file\n"
        );
    }
}

#[test]
fn mk02_directory_globstar_inclusion_below_ignored_directory() {
    directory_glob_inclusion_roundtrips("**/needed.txt");
}

#[test]
fn mk02_directory_partial_component_inclusion_below_ignored_directory() {
    directory_glob_inclusion_roundtrips("private*/needed.txt");
}

#[test]
fn mk02_directory_glob_inclusion_reaches_vault_review() {
    for pattern in ["**/.env", "private*/.env"] {
        let root = test_root("move-directory-glob-review");
        let _cleanup = PlainWorkspaceCleanup(root.clone());
        let directory = root.join("notes");
        fs::create_dir_all(directory.join("private-extra")).unwrap();
        fs::write(directory.join(".gitignore"), "private-extra/\n").unwrap();
        fs::write(directory.join(".charioxignore"), format!("!{pattern}\n")).unwrap();
        fs::write(directory.join("private-extra/.env"), b"APP_TITLE=fixture\n").unwrap();
        let items =
            vault_artifacts::review_items(&BTreeMap::from([("notes".into(), directory)])).unwrap();
        assert!(
            items
                .iter()
                .any(|item| item.path == "notes/private-extra/.env"),
            "glob-selected protected files must reach the source Vault review"
        );
    }
}
