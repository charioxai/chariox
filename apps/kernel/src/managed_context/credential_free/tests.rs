use super::*;

#[test]
fn mp08_mp11_inline_shell_assignments_are_all_inspected() {
    for (index, text) in [
        "FOO=bar API_KEY=synthetic-canary curl https://example.test",
        "echo ready;API_KEY=synthetic-canary curl https://example.test",
        "env FOO=bar API_KEY=synthetic-canary curl https://example.test",
        "FOO=bar API_KEY+=synthetic-canary curl https://example.test",
        "FOO=bar API_\"KEY\"=synthetic-canary curl https://example.test",
        "FOO=bar API_\\KEY=synthetic-canary curl https://example.test",
        "FOO=bar \\\nAPI_KEY=synthetic-canary curl https://example.test",
        "sh -c 'FOO=bar API_KEY=synthetic-canary curl https://example.test'",
        "curl --data 'username=owner&password=synthetic-canary' https://example.test",
    ]
    .iter()
    .enumerate()
    {
        assert!(
            validate_bytes("bootstrap.sh", text.as_bytes()).is_err(),
            "MP-11 fixture {index}"
        );
    }
}

#[test]
fn mp08_mp11_inline_shell_credential_arguments_are_all_inspected() {
    for (index, text) in [
        "curl --user owner:synthetic-canary https://example.test",
        "curl --user=owner:synthetic-canary https://example.test",
        "curl -u owner:synthetic-canary https://example.test",
        "curl -uowner:synthetic-canary https://example.test",
        "curl --proxy-user owner:synthetic-canary https://example.test",
        "curl -Uowner:synthetic-canary https://example.test",
        "curl --us\"er\" owner:synthetic-canary https://example.test",
        "curl --us\\er owner:synthetic-canary https://example.test",
        "curl --oauth2-bearer synthetic-canary https://example.test",
        "curl --cookie session=synthetic-canary https://example.test",
        "curl --header 'Authorization: Basic synthetic-canary' https://example.test",
        "curl --user \"$(read-value)\" https://example.test",
        "curl --user", // MP-11: unsupported/missing credential arguments fail closed.
        "curl --client-secret-file ./public-looking-file https://example.test",
        "curl -fsSu owner:synthetic-canary https://example.test",
        "/usr/bin/curl -fsSUowner:synthetic-canary https://example.test",
        "# curl -u owner:synthetic-canary https://example.test",
        "wget --user owner:synthetic-canary https://example.test",
        "curl --${AUTH_OPTION} owner:synthetic-canary https://example.test",
        "curl $'--us\\x65r' owner:synthetic-canary https://example.test",
        "curl --us\"er owner:synthetic-canary https://example.test",
        "curl --url=https://owner:synthetic-canary@example.test",
    ]
    .iter()
    .enumerate()
    {
        assert!(
            validate_bytes("bootstrap.sh", text.as_bytes()).is_err(),
            "MP-11 fixture {index}"
        );
    }
    for (index, text) in [
        "FOO=bar BAR=baz curl --retry 3 https://example.test",
        "curl --user-agent chariox https://example.test",
        "chromium --user-data-dir /tmp/profile --profile-directory Default",
        "agent --max-tokens 4096 --token-limit=4096",
        "--max-tokens=4096",
        r#"{"args":["--max-tokens=4096"]}"#,
        "printf '%s\\n' 'ordinary project instructions'",
        "set -euo pipefail",
        "python -u worker.py",
        "git add -u",
        "curl -ooutput.txt https://example.test",
        "curl -XPUT https://example.test",
        "pip install --user package",
        "wget -U chariox https://example.test",
        "curl https://example.test\npython -u worker.py",
        "curl -- -u",
    ]
    .iter()
    .enumerate()
    {
        assert!(
            validate_bytes("bootstrap.sh", text.as_bytes()).is_ok(),
            "MP-11 benign shell fixture {index}"
        );
    }
}

// MP-08/MP-11: real development export fixtures, shared by source and target boundaries.
pub(crate) struct InlineShellFixture {
    _cleanup: ScanRoot,
    pub(crate) root: PathBuf,
    pub(crate) project: PathBuf,
}

impl InlineShellFixture {
    pub(crate) fn new(text: &str, history: bool) -> Self {
        Self::file("bootstrap.sh", text.as_bytes(), history)
    }

    pub(crate) fn file(path: &str, bytes: &[u8], history: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-inline-shell-{:032x}",
            rand::random::<u128>()
        ));
        let project = root.join("project");
        fs::create_dir_all(&project).unwrap();
        let fixture = Self {
            _cleanup: ScanRoot(root.clone()),
            root,
            project,
        };
        git(&fixture.project, &["init", "-b", "main"], MAX_FILE).unwrap();
        git(
            &fixture.project,
            &["config", "user.name", "Synthetic fixture"],
            MAX_FILE,
        )
        .unwrap();
        git(
            &fixture.project,
            &["config", "user.email", "fixture@example.test"],
            MAX_FILE,
        )
        .unwrap();
        fs::write(
            fixture.project.join(path),
            if history { bytes } else { b"echo ready\n" },
        )
        .unwrap();
        git(&fixture.project, &["add", path], MAX_FILE).unwrap();
        git(&fixture.project, &["commit", "-m", "fixture"], MAX_FILE).unwrap();
        fs::write(
            fixture.project.join(path),
            if history { b"echo ready\n" } else { bytes },
        )
        .unwrap();
        if history {
            git(&fixture.project, &["add", path], MAX_FILE).unwrap();
            git(
                &fixture.project,
                &["commit", "-m", "replace fixture"],
                MAX_FILE,
            )
            .unwrap();
        }
        fixture
    }

    fn assert_target_refuses(&self) {
        self.assert_target_result(false);
    }

    fn assert_target_result(&self, accepted: bool) {
        use crate::managed_context::{development::*, owner_managed::*, package::*};
        let development = export_development_context(DevelopmentContextExportRequest {
            project_id: "project".into(),
            repositories: vec![DevelopmentRepositorySelection {
                workspace_id: self.project.display().to_string(),
                worktree_id: None,
                worktree_path: self.project.clone(),
                role: DevelopmentRepositoryRole::Primary,
            }],
            archive_path: self.root.join("development.tar.gz"),
        })
        .unwrap();
        let binding = ManagedContextPackageBinding {
            plan: ManagedContextPlanBinding {
                destination: Some(OwnerManagedDestination::OwnerManagedMachine {
                    machine_id: "target-machine".into(),
                    kernel_id: "target-kernel".into(),
                }),
                context_id: "context".into(),
                plan_digest: format!("sha256:{}", "1".repeat(64)),
                kernel_context: ManagedContextKernelSelection::Empty,
                development: ManagedContextDevelopmentSelection::SourceProject {
                    project_id: "project".into(),
                    repositories: vec![DevelopmentSourceRepositoryBinding {
                        role: DevelopmentRepositoryRole::Primary,
                        workspace_id: self.project.display().to_string(),
                        worktree_id: None,
                    }],
                },
                provider_accounts: ManagedContextProviderAccountSelection::None,
                git_credentials: ManagedContextGitCredentialSelection::None,
            },
            target_environment_id: String::new(),
            source_kernel_id: "source-kernel".into(),
            source_key_thumbprint: "a".repeat(64),
            target_kernel_id: "target-kernel".into(),
            target_key_thumbprint: "b".repeat(64),
        };
        // MP-11: compose an attacker-controlled package that bypassed source admission.
        let package = export_managed_context_package(ManagedContextPackageExportRequest {
            plan: binding.plan.clone(),
            target_environment_id: binding.target_environment_id.clone(),
            source_kernel_id: binding.source_kernel_id.clone(),
            source_key_thumbprint: binding.source_key_thumbprint.clone(),
            target_kernel_id: binding.target_kernel_id.clone(),
            target_key_thumbprint: binding.target_key_thumbprint.clone(),
            development: ManagedContextPackageDevelopment::FromSource {
                archive_path: development.archive_path,
                archive_sha256: development.archive_sha256,
            },
            kernel_context: ManagedContextPackageKernel::Empty,
            provider_accounts: ManagedContextPackageProviderAccounts::None,
            git_credentials: ManagedContextPackageGitCredentials::None,
            package_path: self.root.join("context.pkg"),
        })
        .unwrap();
        let target = self.root.join("target");
        let result = apply_managed_context_package(ManagedContextPackageApplicationRequest {
            transfer_id: "transfer".into(),
            package_path: package.package_path,
            expected_package_sha256: package.package_sha256,
            expected_binding: binding,
            development_destination_root: target.clone(),
            target_private_key: "unused".into(),
            project_environment_target: None,
            provider_account_target: None,
            git_credential_target: None,
        });
        if accepted {
            assert!(
                result.is_ok(),
                "MP-08 ordinary shell copy refused: {result:?}"
            );
            assert!(target.exists());
            return;
        }
        let error = result.expect_err("MP-11 target must reject inline shell credentials");
        assert!(error.to_string().contains("credential-free context"));
        assert!(!target.exists(), "MP-11 refusal precedes target mutation");
        assert!(!self.root.join("kernel-context-context").exists());
    }
}

#[test]
fn mp08_mp11_target_refuses_compound_shell_assignments_in_overlay() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    InlineShellFixture::new(
        "FOO=bar API_KEY=synthetic-canary curl https://example.test\n",
        false,
    )
    .assert_target_refuses();
}

#[test]
fn mp08_mp11_target_refuses_basic_auth_in_overlay() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    InlineShellFixture::new(
        "curl --user owner:synthetic-canary https://example.test\n",
        false,
    )
    .assert_target_refuses();
}
#[test]
fn mp11_owner_context_refuses_credentials_and_environment_files() {
    for (path, bytes) in [
        (".env", b"canary-value".as_slice()),
        ("auth.json", b"canary-value"),
        ("file", b"API_KEY=synthetic-canary"),
        ("file", b"export API_KEY=synthetic-canary:with-colon"),
        ("file", b"Authorization: Bearer synthetic-canary"),
        ("file", br#"{"args":["--api-key","synthetic-canary"]}"#),
        ("file", br#"{"x-api-key":"synthetic-canary"}"#),
        ("file", b"-----BEGIN PRIVATE KEY-----"),
        ("file", b"https://user:synthetic-canary@example.test"),
        ("file", b"https://example.test?token=synthetic-canary"),
        (
            "file",
            br#"{"ciphertext":"synthetic-vault-canary","kdf":{}}"#,
        ),
    ] {
        assert!(validate_bytes(path, bytes).is_err(), "fixture path {path}");
    }
    assert!(validate_bytes("README.md", b"ordinary project instructions").is_ok());
    assert!(validate_bytes("origin-url", b"ssh://git@example.test/project.git").is_ok());
    assert!(validate_bytes("file", br#"{"args":["--max-tokens","4096"]}"#).is_ok());
}
#[test]
fn mp08_mp11_owner_package_refuses_credentials_in_git_history() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-owner-history-{:032x}",
        rand::random::<u128>()
    ));
    fs::create_dir_all(root.join("project")).unwrap();
    let _cleanup = ScanRoot(root.clone());
    let project = root.join("project");
    git(&project, &["init", "-b", "main"], MAX_FILE).unwrap();
    git(
        &project,
        &["config", "user.name", "Synthetic fixture"],
        MAX_FILE,
    )
    .unwrap();
    git(
        &project,
        &["config", "user.email", "fixture@example.test"],
        MAX_FILE,
    )
    .unwrap();
    fs::write(
        project.join("README.md"),
        "API_KEY=synthetic-history-canary\n",
    )
    .unwrap();
    git(&project, &["add", "README.md"], MAX_FILE).unwrap();
    git(&project, &["commit", "-m", "fixture"], MAX_FILE).unwrap();
    fs::write(
        project.join("README.md"),
        "Ordinary current project content\n",
    )
    .unwrap();
    git(&project, &["add", "README.md"], MAX_FILE).unwrap();
    git(
        &project,
        &["commit", "-m", "remove fixture value"],
        MAX_FILE,
    )
    .unwrap();
    let archive = super::super::development::export_development_context(
        super::super::development::DevelopmentContextExportRequest {
            project_id: "project".into(),
            repositories: vec![super::super::development::DevelopmentRepositorySelection {
                workspace_id: project.display().to_string(),
                worktree_id: None,
                worktree_path: project,
                role: super::super::development::DevelopmentRepositoryRole::Primary,
            }],
            archive_path: root.join("development.tar.gz"),
        },
    )
    .unwrap();
    assert!(validate_development_archive(&archive.archive_path).is_err());
    assert!(!fs::read_dir(&root).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".owner-context-scan-")));
}

#[test]
fn mp08_mp11_target_refuses_compound_shell_assignments_in_history() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    InlineShellFixture::new(
        "FOO=bar API_KEY=synthetic-canary curl https://example.test\n",
        true,
    )
    .assert_target_refuses();
}

#[test]
fn mp08_mp11_target_refuses_basic_auth_in_history() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    InlineShellFixture::new(
        "curl --user owner:synthetic-canary https://example.test\n",
        true,
    )
    .assert_target_refuses();
}

#[test]
fn mp08_mp11_target_accepts_ordinary_short_flags_in_overlay_and_history() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    for history in [false, true] {
        InlineShellFixture::new(
            "set -euo pipefail\npython -u worker.py\ngit add -u\n",
            history,
        )
        .assert_target_result(true);
    }
}

// MP-08/MP-11: encoded files and package metadata cross both real package boundaries.
pub(crate) fn utf16(text: &str, little_endian: bool) -> Vec<u8> {
    let mut bytes = if little_endian {
        vec![0xff, 0xfe]
    } else {
        vec![0xfe, 0xff]
    };
    for unit in text.encode_utf16() {
        bytes.extend(if little_endian {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        });
    }
    bytes
}

pub(crate) const PACKAGE_METADATA: &[(&str, &[u8])] = &[
    ("package.json", br#"{"dependencies":{"js-tokens":"^4.0.0","secret-tool":"^1.0.0"},"devDependencies":{"@example/tokenizer":"^2.0.0"},"scripts":{"build":"node build.js"}}"#),
    ("package-lock.json", br#"{"lockfileVersion":3,"packages":{"":{"dependencies":{"js-tokens":"^4.0.0"}},"node_modules/js-tokens":{"version":"4.0.0","resolved":"https://registry.npmjs.org/js-tokens/-/js-tokens-4.0.0.tgz","integrity":"sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="}},"dependencies":{"js-tokens":{"version":"4.0.0"}}}"#),
    ("usage.json", br#"{"input_tokens":42,"output_tokens":7,"total_tokens":49,"token_count":49,"max_tokens":4096}"#),
];

#[test]
fn mp08_mp11_target_refuses_utf16_in_overlay_and_history() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    for history in [false, true] {
        for little_endian in [true, false] {
            InlineShellFixture::file(
                "bootstrap.ps1",
                &utf16("$env:API_KEY = 'synthetic-canary'\n", little_endian),
                history,
            )
            .assert_target_result(false);
        }
    }
}

#[test]
fn mp08_mp11_target_accepts_package_metadata_in_overlay_and_history() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    for history in [false, true] {
        for (path, bytes) in PACKAGE_METADATA {
            InlineShellFixture::file(path, bytes, history).assert_target_result(true);
        }
    }
}

#[test]
fn mp11_supported_encodings_are_inspected_and_unclassifiable_bytes_refused() {
    let credential = "$env:API_KEY = 'synthetic-canary'\n";
    let ordinary = "Write-Output 'ready 🦀'\n";
    for text in [credential, ordinary] {
        for little_endian in [true, false] {
            let mut utf32 = if little_endian {
                vec![0xff, 0xfe, 0, 0]
            } else {
                vec![0, 0, 0xfe, 0xff]
            };
            for ch in text.chars() {
                utf32.extend(if little_endian {
                    (ch as u32).to_le_bytes()
                } else {
                    (ch as u32).to_be_bytes()
                });
            }
            for bytes in [
                utf16(text, little_endian),
                utf32,
                [b"\xef\xbb\xbf".as_slice(), text.as_bytes()].concat(),
            ] {
                assert_eq!(
                    validate_bytes("bootstrap.ps1", &bytes).is_ok(),
                    text == ordinary,
                    "MP-11 supported encoding"
                );
            }
        }
    }
    for bytes in [
        b"\xff\xfe\x00".as_slice(),
        b"\xff\xfe\x00\xd8",
        b"\x00\x00\xfe\xff\x00\x11\x00\x00",
        b"\xffAPI_KEY=synthetic-canary",
        b"A\x00P\x00I\x00",
        b"echo \x1bready",
        b"\xff\xfe\x00\x00\x61",
    ] {
        assert!(
            validate_bytes("bootstrap.ps1", bytes).is_err(),
            "MP-11 unclassifiable bytes"
        );
    }
}

#[test]
fn mp11_metadata_roles_do_not_hide_authentication_values() {
    for (path, bytes) in [
        (
            "package.json",
            br#"{"dependencies":{"js-tokens":"https://owner:synthetic-canary@example.test"}}"#
                .as_slice(),
        ),
        (
            "package.json",
            br#"{"dependencies":{"js-tokens":{"api_key":"synthetic-canary"}}}"#,
        ),
        (
            "package-lock.json",
            br#"{"packages":{"node_modules/js-tokens":{"token":"synthetic-canary"}}}"#,
        ),
        (
            "package-lock.json",
            br#"{"packages":{"node_modules/js-tokens":{"integrity":"API_KEY=synthetic-canary"}}}"#,
        ),
        (
            "package.json",
            br#"{"scripts":{"build":"API_KEY=synthetic-canary node build.js"}}"#,
        ),
        (
            "package.json",
            br#"{"token":"synthetic-canary","input_tokens":4}"#,
        ),
        (
            "package.json",
            br#"{"dependencies":{"js-tokens":{"dependencies":{"token":"synthetic-canary"}}}}"#,
        ),
        (
            "settings.json",
            br#"{"vault_file_base64":"synthetic-canary"}"#,
        ),
        (
            "settings.json",
            br#"{"sealed_unlock_key":"synthetic-canary"}"#,
        ),
        ("usage.json", br#"{"token_count":"synthetic-canary"}"#),
        ("usage.json", br#"{"token":1234}"#),
        (
            "settings.json",
            br#"{"dependencies":{"token":"synthetic-canary"}}"#,
        ),
        (
            "package.json",
            br#"{"dependencies":{"js-tokens":"^4.0.0"},"extra":{"secret":"synthetic-canary"}}"#,
        ),
        (
            "package.json",
            br#"{"name":"js-tokens","description":"-----BEGIN PRIVATE KEY-----"}"#,
        ),
        (
            "package.json",
            br#"{"dependencies":{"js-tokens":"^4.0.0"} // malformed"#,
        ),
    ] {
        assert!(
            validate_bytes(path, bytes).is_err(),
            "MP-11 authentication fixture {path}"
        );
    }
}
