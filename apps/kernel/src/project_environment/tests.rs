use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::policy::{MAX_LOCKFILE_BYTES, MAX_TOTAL_REPOSITORY_BYTES};
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

fn manifest_for_lockfiles(paths: &[&str]) -> String {
    let lockfiles = paths
        .iter()
        .map(|path| format!("\"{path}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("schema_version = 1\nlockfiles = [{lockfiles}]\n")
}

fn write_sized_file(path: &Path, size: usize) {
    fs::write(path, vec![b'x'; size]).expect("bounded fixture file should be written");
}

fn write_aggregate_fixture(root: &Path, manifest_len: usize, adjustment: isize) -> usize {
    let paths = ["one.lock", "two.lock", "three.lock"];
    let remaining = MAX_TOTAL_REPOSITORY_BYTES - manifest_len;
    let first = remaining / 3;
    let second = remaining / 3;
    let mut third = remaining - first - second;
    if adjustment.is_negative() {
        third -= adjustment.unsigned_abs();
    } else {
        third += adjustment as usize;
    }
    write_sized_file(&root.join(paths[0]), first);
    write_sized_file(&root.join(paths[1]), second);
    write_sized_file(&root.join(paths[2]), third);
    manifest_len + first + second + third
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
fn boundary_controls_are_rejected_before_normalization() {
    let digest = "11".repeat(32);
    let mut newline_digest = identity();
    newline_digest.repository_digest = format!("{digest}\n");
    assert!(matches!(
        newline_digest.validate_and_normalize(),
        Err(ProjectEnvironmentError::InvalidField { .. })
    ));

    let mut newline_policy = identity();
    newline_policy.policy_version = format!("{POLICY_VERSION}\r");
    assert!(matches!(
        newline_policy.validate_and_normalize(),
        Err(ProjectEnvironmentError::InvalidField { .. })
    ));

    let path_control = RepositoryManifest::from_toml(
        r#"
schema_version = 1
lockfiles = ["Cargo.lock\n"]
"#,
    );
    assert!(matches!(
        path_control,
        Err(ProjectEnvironmentError::InvalidField { .. })
    ));

    let digest_control = RepositoryManifest::from_toml(&format!(
        "schema_version = 1\n[[validation]]\nname = \"digest\"\nkind = \"file_digest\"\npath = \"Cargo.lock\"\nexpected_digest = \"{digest}\\r\"\n"
    ));
    assert!(matches!(
        digest_control,
        Err(ProjectEnvironmentError::InvalidField { .. })
    ));

    assert!(RepositoryManifest::from_toml(&manifest_toml("1.85.0")).is_ok());
    assert!(identity().validate_and_normalize().is_ok());
}

#[test]
fn path_normalization_rejects_escape_forms_and_accepts_nested_relative_paths() {
    for path in [
        "../Cargo.lock",
        "/tmp/Cargo.lock",
        "~/Cargo.lock",
        "nested//Cargo.lock",
        "nested/./Cargo.lock",
        "nested/../Cargo.lock",
        "nested\\Cargo.lock",
        "C:/Cargo.lock",
    ] {
        let manifest = format!("schema_version = 1\nlockfiles = ['{path}']\n");
        assert!(
            matches!(
                RepositoryManifest::from_toml(&manifest),
                Err(ProjectEnvironmentError::InvalidPath { .. })
            ),
            "path should be rejected: {path}"
        );
    }

    let valid =
        RepositoryManifest::from_toml("schema_version = 1\nlockfiles = [\"nested/Cargo.lock\"]\n");
    assert!(valid.is_ok());
}

#[test]
fn aggregate_repository_bytes_are_bounded_at_the_explicit_limit() {
    let root = unique_root("aggregate");
    let manifest = manifest_for_lockfiles(&["one.lock", "two.lock", "three.lock"]);
    write_manifest(&root, &manifest);

    let below_total = write_aggregate_fixture(&root, manifest.len(), -1);
    assert_eq!(below_total, MAX_TOTAL_REPOSITORY_BYTES - 1);
    assert!(discover(&root, identity()).is_ok());

    let equal_total = write_aggregate_fixture(&root, manifest.len(), 0);
    assert_eq!(equal_total, MAX_TOTAL_REPOSITORY_BYTES);
    assert!(discover(&root, identity()).is_ok());

    let above_total = write_aggregate_fixture(&root, manifest.len(), 1);
    assert_eq!(above_total, MAX_TOTAL_REPOSITORY_BYTES + 1);
    assert!(above_total > MAX_LOCKFILE_BYTES);
    assert!(matches!(
        discover(&root, identity()),
        Err(ProjectEnvironmentError::BoundExceeded {
            kind: "repository aggregate bytes",
            ..
        })
    ));
}

#[test]
fn discovery_requires_the_manifest_at_the_canonical_location() {
    let missing_root = unique_root("missing-manifest");
    assert!(matches!(
        discover(&missing_root, identity()),
        Err(ProjectEnvironmentError::MissingFile { .. })
    ));

    let misplaced_root = unique_root("misplaced-manifest");
    fs::write(
        misplaced_root.join("environment.toml"),
        manifest_toml("1.85.0"),
    )
    .expect("misplaced manifest should be written");
    assert!(matches!(
        discover(&misplaced_root, identity()),
        Err(ProjectEnvironmentError::MissingFile { .. })
    ));
}

#[test]
fn reordered_inputs_have_the_same_discovered_fingerprint() {
    let first_root = unique_root("reordered-first");
    write_manifest(
        &first_root,
        &manifest_for_lockfiles(&["one.lock", "two.lock"]),
    );
    fs::write(first_root.join("one.lock"), "one\n").expect("first lockfile should be written");
    fs::write(first_root.join("two.lock"), "two\n").expect("second lockfile should be written");

    let second_root = unique_root("reordered-second");
    write_manifest(
        &second_root,
        &manifest_for_lockfiles(&["two.lock", "one.lock"]),
    );
    fs::write(second_root.join("one.lock"), "one\n").expect("first lockfile should be written");
    fs::write(second_root.join("two.lock"), "two\n").expect("second lockfile should be written");

    let rust = identity().toolchain_inputs[0].clone();
    let node = ToolchainInput {
        name: ToolchainKind::Node,
        version: "22.0.0".to_owned(),
        target: None,
        components: vec!["corepack".to_owned()],
    };
    let mut first_identity = identity();
    first_identity.toolchain_inputs = vec![rust.clone(), node.clone()];
    let mut second_identity = identity();
    second_identity.toolchain_inputs = vec![node, rust];

    let first = discover(&first_root, first_identity).expect("first order should discover");
    let second = discover(&second_root, second_identity).expect("second order should discover");
    assert_eq!(first.manifest_digest, second.manifest_digest);
    assert_eq!(first.lockfile_digests, second.lockfile_digests);
    assert_eq!(first.fingerprint, second.fingerprint);
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

    let duplicate_validation_name = RepositoryManifest::from_toml(
        r#"
schema_version = 1
[[validation]]
name = "same"
kind = "path_exists"
path = "one.txt"

[[validation]]
name = "same"
kind = "path_exists"
path = "two.txt"
"#,
    );
    assert!(matches!(
        duplicate_validation_name,
        Err(ProjectEnvironmentError::DuplicateEntry { .. })
    ));

    let duplicate_validation_path = RepositoryManifest::from_toml(
        r#"
schema_version = 1
[[validation]]
name = "first"
kind = "path_exists"
path = "same.txt"

[[validation]]
name = "second"
kind = "path_exists"
path = "same.txt"
"#,
    );
    assert!(matches!(
        duplicate_validation_path,
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

#[cfg(unix)]
#[test]
fn opened_files_reject_final_and_intermediate_replacement() {
    use std::os::unix::fs::symlink;

    let (root, _) = fixture("replacement-final");
    let canonical_root =
        super::discovery::canonical_repository_root(&root).expect("root should be canonicalized");
    let final_path = root.join("Cargo.lock");
    let opened = super::discovery::open_declared_file(&canonical_root, "Cargo.lock", &final_path)
        .expect("final file should open safely");
    fs::rename(&final_path, root.join("Cargo.lock.original"))
        .expect("original final file should be moved");
    fs::write(&final_path, "replacement\n").expect("replacement final file should be written");
    assert!(matches!(
        super::discovery::verify_opened_file(&canonical_root, &final_path, &opened),
        Err(ProjectEnvironmentError::ConcurrentReplacement { .. })
    ));

    let nested = root.join("nested");
    fs::create_dir(&nested).expect("nested directory should be created");
    let nested_path = nested.join("Cargo.lock");
    fs::write(&nested_path, "nested\n").expect("nested file should be written");
    let nested_opened =
        super::discovery::open_declared_file(&canonical_root, "nested/Cargo.lock", &nested_path)
            .expect("nested file should open safely");

    let moved_nested = root.join("nested.original");
    fs::rename(&nested, &moved_nested).expect("original nested directory should be moved");
    let outside = unique_root("replacement-outside");
    fs::write(outside.join("Cargo.lock"), "outside\n")
        .expect("outside replacement target should be written");
    symlink(&outside, &nested).expect("intermediate replacement symlink should be created");
    assert!(matches!(
        super::discovery::verify_opened_file(&canonical_root, &nested_path, &nested_opened),
        Err(ProjectEnvironmentError::PathOutsideRoot { .. }
            | ProjectEnvironmentError::SymlinkPath { .. }
            | ProjectEnvironmentError::ConcurrentReplacement { .. })
    ));

    let inside_target = root.join("inside-target.lock");
    fs::write(&inside_target, "inside\n").expect("inside target should be written");
    let final_symlink = root.join("final-symlink.lock");
    symlink(&inside_target, &final_symlink).expect("final replacement symlink should be created");
    assert!(matches!(
        super::discovery::open_declared_file(&canonical_root, "final-symlink.lock", &final_symlink),
        Err(ProjectEnvironmentError::SymlinkPath { .. })
    ));
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
