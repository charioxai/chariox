use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    discover, EnvironmentIdentity, ProjectEnvironmentError, RepositoryManifest, TargetKernelRealm,
    ToolchainInput, ToolchainKind, MANIFEST_RELATIVE_PATH, MAX_MANIFEST_BYTES, POLICY_VERSION,
};

fn unique_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock should be after the epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "chariox-project-environment-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("test repository root should be created");
    root
}

fn identity() -> EnvironmentIdentity {
    EnvironmentIdentity {
        owner_id: "owner-a".to_owned(),
        project_id: "project-a".to_owned(),
        workspace_id: "workspace-a".to_owned(),
        worktree_id: "worktree-a".to_owned(),
        target: TargetKernelRealm {
            kernel_id: "kernel-a".to_owned(),
            realm: "local".to_owned(),
        },
        repository_digest: "11".repeat(32),
        policy_version: POLICY_VERSION.to_owned(),
        toolchain_inputs: vec![ToolchainInput {
            name: ToolchainKind::Rust,
            version: "1.85.0".to_owned(),
            target: Some("x86_64-unknown-linux-gnu".to_owned()),
            components: vec!["rustfmt".to_owned()],
        }],
    }
}

fn manifest_toml(version: &str) -> String {
    format!(
        r#"
schema_version = 1
lockfiles = ["Cargo.lock"]

[[utilities]]
name = "git"
version = "2.45.0"
required = true

[[toolchains]]
name = "rust"
version = "{version}"
target = "x86_64-unknown-linux-gnu"
components = ["rustfmt"]

[[validation]]
name = "instruction-file-is-data"
kind = "path_exists"
path = "AGENTS.md"

[[validation]]
name = "script-file-is-data"
kind = "path_exists"
path = "setup.sh"
"#
    )
}

fn write_manifest(root: &Path, contents: &str) {
    fs::write(root.join(MANIFEST_RELATIVE_PATH), contents).expect("manifest should be written");
}

fn fixture(label: &str) -> (PathBuf, EnvironmentIdentity) {
    let root = unique_root(label);
    write_manifest(&root, &manifest_toml("1.85.0"));
    fs::write(root.join("Cargo.lock"), "lockfile-v1\n").expect("lockfile should be written");
    fs::write(
        root.join("AGENTS.md"),
        "touch executed-by-instruction-file\n",
    )
    .expect("instruction file should be written");
    fs::write(root.join("setup.sh"), "touch executed-by-script\n")
        .expect("script file should be written");
    (root, identity())
}

#[test]
fn fingerprint_changes_for_each_isolation_and_content_input() {
    let (root, base_identity) = fixture("fingerprint");
    let baseline = discover(&root, base_identity.clone()).expect("baseline should discover");

    let cases = [
        ("owner", {
            let mut value = base_identity.clone();
            value.owner_id = "owner-b".to_owned();
            value
        }),
        ("project", {
            let mut value = base_identity.clone();
            value.project_id = "project-b".to_owned();
            value
        }),
        ("workspace", {
            let mut value = base_identity.clone();
            value.workspace_id = "workspace-b".to_owned();
            value
        }),
        ("worktree", {
            let mut value = base_identity.clone();
            value.worktree_id = "worktree-b".to_owned();
            value
        }),
        ("kernel", {
            let mut value = base_identity.clone();
            value.target.kernel_id = "kernel-b".to_owned();
            value
        }),
        ("realm", {
            let mut value = base_identity.clone();
            value.target.realm = "remote".to_owned();
            value
        }),
        ("repository", {
            let mut value = base_identity.clone();
            value.repository_digest = "22".repeat(32);
            value
        }),
        ("toolchain", {
            let mut value = base_identity.clone();
            value.toolchain_inputs[0].version = "1.86.0".to_owned();
            value
        }),
    ];
    for (label, changed_identity) in cases {
        let changed = discover(&root, changed_identity)
            .unwrap_or_else(|error| panic!("{label} input should discover: {error}"));
        assert_ne!(
            baseline.fingerprint, changed.fingerprint,
            "{label} must be fingerprint-bound"
        );
    }

    fs::write(root.join("Cargo.lock"), "lockfile-v2\n").expect("lockfile should be changed");
    let changed_lockfile =
        discover(&root, base_identity.clone()).expect("changed lockfile should discover");
    assert_ne!(baseline.fingerprint, changed_lockfile.fingerprint);

    write_manifest(&root, &manifest_toml("1.86.0"));
    let changed_manifest =
        discover(&root, base_identity.clone()).expect("changed manifest should discover");
    assert_ne!(baseline.fingerprint, changed_manifest.fingerprint);
    assert_ne!(baseline.manifest_digest, changed_manifest.manifest_digest);

    let mut unsupported_policy = base_identity;
    unsupported_policy.policy_version = "project-environment-policy-v2".to_owned();
    assert!(matches!(
        discover(&root, unsupported_policy),
        Err(ProjectEnvironmentError::UnsupportedPolicyVersion(_))
    ));
}

#[test]
fn canonical_manifest_hash_is_stable_across_order_and_normalization() {
    let first = RepositoryManifest::from_toml(
        r#"
schema_version = 1
lockfiles = ["Cargo.lock", "package-lock.json"]

[[utilities]]
name = "git"
version = "2.45.0"

[[utilities]]
name = "cargo"
version = "1.85.0"

[[toolchains]]
name = "rust"
version = " 1.85.0 "
components = ["rustfmt", "clippy"]
"#,
    )
    .expect("first manifest should parse");
    let second = RepositoryManifest::from_toml(
        r#"
schema_version = 1
lockfiles = ["package-lock.json", "Cargo.lock"]

[[utilities]]
name = "cargo"
version = "1.85.0"

[[utilities]]
name = "git"
version = "2.45.0"

[[toolchains]]
name = "rust"
version = "1.85.0"
components = ["clippy", "rustfmt"]
"#,
    )
    .expect("second manifest should parse");
    assert_eq!(first, second);
    assert_eq!(first.digest(), second.digest());
}

#[test]
fn malformed_oversized_traversal_duplicate_and_unsupported_entries_are_rejected() {
    let malformed = RepositoryManifest::from_toml("schema_version = [");
    assert!(matches!(
        malformed,
        Err(ProjectEnvironmentError::MalformedManifest { .. })
    ));

    let oversized = format!("schema_version = 1\n#{}\n", "x".repeat(MAX_MANIFEST_BYTES));
    assert!(matches!(
        RepositoryManifest::from_toml(&oversized),
        Err(ProjectEnvironmentError::BoundExceeded { .. })
    ));

    let traversal = RepositoryManifest::from_toml(
        r#"
schema_version = 1
lockfiles = ["../Cargo.lock"]
"#,
    );
    assert!(matches!(
        traversal,
        Err(ProjectEnvironmentError::InvalidPath { .. })
    ));

    let duplicate = RepositoryManifest::from_toml(
        r#"
schema_version = 1
lockfiles = ["Cargo.lock", "Cargo.lock"]
"#,
    );
    assert!(matches!(
        duplicate,
        Err(ProjectEnvironmentError::DuplicateEntry { .. })
    ));

    let unsupported_utility = RepositoryManifest::from_toml(
        r#"
schema_version = 1
[[utilities]]
name = "curl"
version = "8.0.0"
"#,
    );
    assert!(matches!(
        unsupported_utility,
        Err(ProjectEnvironmentError::MalformedManifest { .. })
    ));

    let unsupported_toolchain = RepositoryManifest::from_toml(
        r#"
schema_version = 1
[[toolchains]]
name = "java"
version = "21"
"#,
    );
    assert!(matches!(
        unsupported_toolchain,
        Err(ProjectEnvironmentError::MalformedManifest { .. })
    ));

    let free_form_command = RepositoryManifest::from_toml(
        r#"
schema_version = 1
[[validation]]
name = "unsafe"
kind = "path_exists"
path = "setup.sh"
command = "touch executed"
"#,
    );
    assert!(matches!(
        free_form_command,
        Err(ProjectEnvironmentError::MalformedManifest { .. })
    ));
}

#[test]
fn discovery_rejects_manifest_lockfile_and_probe_symlinks() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let root = unique_root("symlink-lockfile");
        write_manifest(
            &root,
            &manifest_toml("1.85.0").replace("Cargo.lock", "link.lock"),
        );
        fs::write(root.join("AGENTS.md"), "data\n").expect("instruction file should be written");
        fs::write(root.join("setup.sh"), "data\n").expect("script should be written");
        let base_identity = identity();
        let outside = unique_root("symlink-target").join("outside.lock");
        fs::write(&outside, "outside\n").expect("outside lockfile should be written");
        symlink(&outside, root.join("link.lock")).expect("lockfile symlink should be created");
        assert!(matches!(
            discover(&root, base_identity.clone()),
            Err(ProjectEnvironmentError::SymlinkPath { .. })
        ));

        let root = unique_root("symlink-manifest");
        fs::write(root.join("Cargo.lock"), "lockfile\n").expect("lockfile should be written");
        fs::write(root.join("AGENTS.md"), "data\n").expect("instruction file should be written");
        fs::write(root.join("setup.sh"), "data\n").expect("script should be written");
        let base_identity = identity();
        let outside = unique_root("manifest-target").join("manifest.toml");
        fs::write(&outside, manifest_toml("1.85.0")).expect("outside manifest should be written");
        symlink(&outside, root.join(MANIFEST_RELATIVE_PATH))
            .expect("manifest symlink should be created");
        assert!(matches!(
            discover(&root, base_identity),
            Err(ProjectEnvironmentError::SymlinkPath { .. })
        ));

        let root = unique_root("symlink-probe");
        write_manifest(&root, &manifest_toml("1.85.0"));
        fs::write(root.join("Cargo.lock"), "lockfile\n").expect("lockfile should be written");
        fs::write(root.join("setup.sh"), "data\n").expect("script should be written");
        let base_identity = identity();
        let outside = unique_root("probe-target").join("instruction.md");
        fs::write(&outside, "not executable\n").expect("outside probe should be written");
        symlink(&outside, root.join("AGENTS.md")).expect("probe symlink should be created");
        assert!(matches!(
            discover(&root, base_identity),
            Err(ProjectEnvironmentError::SymlinkPath { .. })
        ));
    }
}

#[test]
fn instruction_files_and_scripts_are_observed_as_data_and_never_executed() {
    let (root, base_identity) = fixture("non-execution");
    let marker = root.join("executed-by-instruction-file");
    let script_marker = root.join("executed-by-script");
    assert!(!marker.exists());
    assert!(!script_marker.exists());

    let contract = discover(&root, base_identity).expect("data-only discovery should succeed");
    assert_eq!(contract.manifest.validation.len(), 2);
    assert!(!marker.exists(), "AGENTS.md must never be executed");
    assert!(!script_marker.exists(), "setup.sh must never be executed");
}
