use super::*;

// MP-08/MP-11: every shell text fixture also exercises printable asset runs.
fn shell_text_cases() -> Vec<(&'static str, bool)> {
    let mut cases = Vec::new();
    cases.extend(
        [
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
        .into_iter()
        .map(|text| (text, false)),
    );
    cases.extend(
        [
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
        .into_iter()
        .map(|text| (text, false)),
    );
    cases.extend(
        [
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
        .into_iter()
        .map(|text| (text, true)),
    );
    cases
}

#[test]
fn mp08_mp11_inline_shell_credentials_are_all_inspected() {
    for (index, (text, data)) in shell_text_cases().into_iter().enumerate() {
        assert_eq!(
            validate_bytes("bootstrap.sh", text.as_bytes()).is_ok(),
            data,
            "MP-08/MP-11 shell text fixture {index}"
        );
    }
}

// MP-08/MP-11 #903 r21: every text-path table fixture, including the complete
// shell command/assignment table, is inspected identically in a PNG and font.
#[test]
fn mp08_mp11_every_text_fixture_has_asset_parity() {
    let mut fixtures = classification_cases()
        .into_iter()
        .filter(|(path, bytes, _)| matches!(text::classify(path, bytes), text::Content::Text(_)))
        .map(|(path, bytes, _)| (path, bytes))
        .collect::<Vec<_>>();
    fixtures.extend(
        shell_text_cases()
            .into_iter()
            .map(|(text, _)| ("bootstrap.sh", text.as_bytes().to_vec())),
    );
    fixtures.extend(
        metadata_authentication_cases()
            .into_iter()
            .map(|(path, bytes)| (path, bytes.to_vec())),
    );
    fixtures.extend(
        guard_text_cases()
            .into_iter()
            .map(|(path, bytes, _)| (path, bytes.to_vec())),
    );
    fixtures.extend(
        public_text_cases()
            .into_iter()
            .map(|(path, bytes, _)| (path, bytes)),
    );
    let mut failures = Vec::new();
    for (index, (original_path, bytes)) in fixtures.into_iter().enumerate() {
        for (path, signature) in ASSET_SIGNATURES {
            // Role is the asset's path, just as if its contents were plain text.
            let plain = validate_bytes(path, &bytes).is_ok();
            let asset = [*signature, b"\0", bytes.as_slice(), b"\0"].concat();
            if validate_bytes(path, &asset).is_ok() != plain {
                failures.push((index, original_path, *path, plain));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "MP-08/MP-11 plain/asset verdict mismatches: {failures:?}"
    );
}

const ASSET_SIGNATURES: &[(&str, &[u8])] = &[
    ("artwork.png", b"\x89PNG\r\n\x1a\n\0\xff"),
    ("font.woff2", b"wOF2\0\x01\0\0\xff"),
];

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
        // The other revision holds ordinary content valid for the path's policy.
        let ordinary: &[u8] = if path.ends_with(".json") {
            b"{}\n"
        } else {
            b"echo ready\n"
        };
        fs::write(
            fixture.project.join(path),
            if history { bytes } else { ordinary },
        )
        .unwrap();
        git(&fixture.project, &["add", path], MAX_FILE).unwrap();
        git(&fixture.project, &["commit", "-m", "fixture"], MAX_FILE).unwrap();
        fs::write(
            fixture.project.join(path),
            if history { ordinary } else { bytes },
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

    fn historical_path(blocked: &str, rename: bool) -> Self {
        let fixture = Self::file("README.txt", b"ordinary opaque fixture value\n", false);
        fs::write(
            fixture.project.join(blocked),
            b"ordinary opaque fixture value\n",
        )
        .unwrap();
        git(&fixture.project, &["add", "."], MAX_FILE).unwrap();
        git(
            &fixture.project,
            &["commit", "-m", "shared historical blob"],
            MAX_FILE,
        )
        .unwrap();
        if rename {
            git(&fixture.project, &["rm", "README.txt"], MAX_FILE).unwrap();
            git(&fixture.project, &["mv", blocked, "README.txt"], MAX_FILE).unwrap();
        } else {
            git(&fixture.project, &["rm", blocked], MAX_FILE).unwrap();
        }
        git(
            &fixture.project,
            &["commit", "-m", "public current filename"],
            MAX_FILE,
        )
        .unwrap();
        fixture
    }

    fn export_archive(
        &self,
        extra_ref: Option<(&str, bool, bool)>,
    ) -> crate::managed_context::development::DevelopmentContextExportResult {
        use crate::managed_context::development::*;
        use sha2::{Digest, Sha256};
        let mut development = export_development_context(DevelopmentContextExportRequest {
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
        let Some((reference, secret, hide_ref)) = extra_ref else {
            return development;
        };
        git(
            &self.project,
            &["checkout", "--orphan", "unrelated"],
            MAX_FILE,
        )
        .unwrap();
        git(&self.project, &["rm", "-rf", "."], MAX_FILE).unwrap();
        fs::write(
            self.project.join(if secret { ".env" } else { "notes.txt" }),
            if secret {
                b"API_KEY=synthetic-canary\n".as_slice()
            } else {
                b"ordinary extra ref\n".as_slice()
            },
        )
        .unwrap();
        git(&self.project, &["add", "."], MAX_FILE).unwrap();
        git(
            &self.project,
            &["commit", "-m", "unrelated fixture"],
            MAX_FILE,
        )
        .unwrap();
        let hidden = git(&self.project, &["rev-parse", "HEAD"], MAX_FILE).unwrap();
        let hidden = std::str::from_utf8(&hidden).unwrap().trim();
        git(&self.project, &["update-ref", reference, hidden], MAX_FILE).unwrap();
        git(&self.project, &["checkout", "main"], MAX_FILE).unwrap();
        git(&self.project, &["branch", "-D", "unrelated"], MAX_FILE).unwrap();
        let bundle = self.root.join("hostile.bundle");
        git(
            &self.project,
            &[
                "bundle",
                "create",
                bundle.to_str().unwrap(),
                "HEAD",
                reference,
            ],
            MAX_FILE,
        )
        .unwrap();
        if hide_ref {
            // Leave its entire pack in place but advertise only the clean HEAD.
            let bytes = fs::read(&bundle).unwrap();
            let end = bytes.windows(2).position(|pair| pair == b"\n\n").unwrap();
            let suffix = format!(" {reference}\n");
            let mut rewritten = Vec::new();
            for line in bytes[..end + 1].split_inclusive(|byte| *byte == b'\n') {
                if !line.ends_with(suffix.as_bytes()) {
                    rewritten.extend_from_slice(line);
                }
            }
            rewritten.push(b'\n');
            rewritten.extend_from_slice(&bytes[end + 2..]);
            fs::write(&bundle, rewritten).unwrap();
        }
        // Prove that clone imports the hidden object while omitting its ref.
        let proof = self.root.join("proof");
        git(
            &self.root,
            &[
                "clone",
                "--bare",
                bundle.to_str().unwrap(),
                proof.to_str().unwrap(),
            ],
            MAX_FILE,
        )
        .unwrap();
        git(&proof, &["cat-file", "-e", hidden], MAX_FILE).unwrap();
        let reachable = git(&proof, &["rev-list", "--all"], MAX_FILE).unwrap();
        assert!(!std::str::from_utf8(&reachable)
            .unwrap()
            .lines()
            .any(|oid| oid == hidden));
        let bundle = fs::read(bundle).unwrap();
        let repository = &mut development.manifest.repositories[0];
        repository.bundle_sha256 = format!("{:x}", Sha256::digest(&bundle));
        repository.bundle_size_bytes = bundle.len() as u64;
        let bundle_path = repository.bundle_path.clone();
        let manifest = serde_json::to_vec(&development.manifest).unwrap();
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(
            File::open(&development.archive_path).unwrap(),
        ));
        let entries: Vec<_> = archive
            .entries()
            .unwrap()
            .map(|entry| {
                let mut entry = entry.unwrap();
                let name = entry.path().unwrap().to_string_lossy().into_owned();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                (name, bytes)
            })
            .collect();
        let encoder = flate2::write::GzEncoder::new(
            File::create(&development.archive_path).unwrap(),
            flate2::Compression::default(),
        );
        let mut archive = tar::Builder::new(encoder);
        for (name, bytes) in entries {
            let bytes = if name == "manifest.json" {
                &manifest
            } else if name == bundle_path {
                &bundle
            } else {
                &bytes
            };
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            archive
                .append_data(&mut header, name, bytes.as_slice())
                .unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();
        let bytes = fs::read(&development.archive_path).unwrap();
        development.archive_sha256 = format!("{:x}", Sha256::digest(&bytes));
        development.archive_size_bytes = bytes.len() as u64;
        development
    }

    fn assert_source_refuses(&self) {
        use crate::managed_context::development::*;
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
        let error = validate_development_archive(&development.archive_path, "project", None)
            .expect_err("source must inspect every historical filename of a shared blob");
        assert!(error.to_string().contains("credential-free context"));
    }

    fn assert_target_refuses(&self) {
        self.assert_target_result(false);
    }

    fn assert_target_result(&self, accepted: bool) {
        self.assert_target_result_with_extra_ref(accepted, None);
    }

    fn assert_target_result_with_extra_ref(
        &self,
        accepted: bool,
        extra_ref: Option<(&str, bool, bool)>,
    ) {
        self.assert_target_archive(accepted, &self.export_archive(extra_ref));
    }

    // MP-11: a correctly hashed archive whose gzip stream ends after the
    // manifest member; the declared artifacts follow in a second member.
    fn two_member_archive(
        &self,
    ) -> crate::managed_context::development::DevelopmentContextExportResult {
        use sha2::{Digest, Sha256};
        use std::io::Write;
        let mut development = self.export_archive(None);
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(
            File::open(&development.archive_path).unwrap(),
        ));
        let mut builder = tar::Builder::new(Vec::new());
        let mut split = 0;
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let name = entry.path().unwrap().to_string_lossy().into_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            builder
                .append_data(&mut header, &name, bytes.as_slice())
                .unwrap();
            if name == "manifest.json" {
                split = builder.get_ref().len();
            }
        }
        let tar = builder.into_inner().unwrap();
        let mut file = File::create(&development.archive_path).unwrap();
        for member in [&tar[..split], &tar[split..]] {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(member).unwrap();
            file.write_all(&encoder.finish().unwrap()).unwrap();
        }
        drop(file);
        let bytes = fs::read(&development.archive_path).unwrap();
        development.archive_sha256 = format!("{:x}", Sha256::digest(&bytes));
        development.archive_size_bytes = bytes.len() as u64;
        development
    }

    fn assert_target_archive(
        &self,
        accepted: bool,
        development: &crate::managed_context::development::DevelopmentContextExportResult,
    ) {
        let result = self.target_archive_result(development);
        let target = self.root.join("target");
        if accepted {
            assert!(
                result.is_ok(),
                "MP-08 ordinary shell copy refused: {result:?}"
            );
            assert!(target.exists());
        } else {
            let error = result.expect_err("MP-11 target must reject inline shell credentials");
            assert!(error.to_string().contains("credential-free context"));
            assert!(!target.exists(), "MP-11 refusal precedes target mutation");
            assert!(!self.root.join("kernel-context-context").exists());
        }
    }
    fn target_archive_result(
        &self,
        development: &crate::managed_context::development::DevelopmentContextExportResult,
    ) -> Result<crate::managed_context::package::ManagedContextPackageImportReceipt, DaemonError>
    {
        use crate::managed_context::{development::*, owner_managed::*, package::*};
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
                archive_path: development.archive_path.clone(),
                archive_sha256: development.archive_sha256.clone(),
            },
            kernel_context: ManagedContextPackageKernel::Empty,
            provider_accounts: ManagedContextPackageProviderAccounts::None,
            git_credentials: ManagedContextPackageGitCredentials::None,
            package_path: self.root.join("context.pkg"),
        })
        .unwrap();
        let target = self.root.join("target");
        apply_managed_context_package(ManagedContextPackageApplicationRequest {
            transfer_id: "transfer".into(),
            package_path: package.package_path,
            expected_package_sha256: package.package_sha256,
            expected_binding: binding,
            development_destination_root: target.clone(),
            target_private_key: "unused".into(),
            project_environment_target: None,
            provider_account_target: None,
            git_credential_target: None,
        })
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
fn guard_text_cases() -> Vec<(&'static str, &'static [u8], bool)> {
    let mut cases = [
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
    ]
    .into_iter()
    .map(|(path, bytes)| (path, bytes, false))
    .collect::<Vec<_>>();
    cases.extend([
        (
            "README.md",
            b"ordinary project instructions".as_slice(),
            true,
        ),
        ("origin-url", b"ssh://git@example.test/project.git", true),
        ("file", br#"{"args":["--max-tokens","4096"]}"#, true),
        ("file", b"a$b=c", false), // Short runs receive the full text policy.
    ]);
    cases
}

#[test]
fn mp11_owner_context_refuses_credentials_and_environment_files() {
    for (path, bytes, data) in guard_text_cases() {
        assert_eq!(
            validate_bytes(path, bytes).is_ok(),
            data,
            "MP-08/MP-11 guard fixture {path}"
        );
    }
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
    assert!(validate_development_archive(&archive.archive_path, "project", None).is_err());
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

pub(crate) fn utf32(text: &str, little_endian: bool) -> Vec<u8> {
    let mut bytes = if little_endian {
        vec![0xff, 0xfe, 0, 0]
    } else {
        vec![0, 0, 0xfe, 0xff]
    };
    for ch in text.chars() {
        bytes.extend(if little_endian {
            (ch as u32).to_le_bytes()
        } else {
            (ch as u32).to_be_bytes()
        });
    }
    bytes
}

// MP-08/MP-11: one malformed variant per way a text file can fail strict
// classification: NUL, invalid UTF-8, a trailing partial UTF-8 sequence, a
// UTF-8 BOM with a NUL, BOM-less UTF-16, UTF-16/32 with a NUL, invalid or
// partial unit, a byte-order mismatch, and UTF-8 bytes after a UTF-16 BOM.
pub(crate) fn malformed(text: &str) -> Vec<Vec<u8>> {
    let bytes = text.as_bytes();
    vec![
        [bytes, b"\0"].concat(),
        [bytes, b"\xff"].concat(),
        [bytes, b"\xe2\x82"].concat(),
        [b"\xef\xbb\xbf", bytes, b"\0"].concat(),
        text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        [utf16(text, true).as_slice(), &[0, 0]].concat(),
        [utf16(text, false).as_slice(), &[0xd8, 0x00]].concat(),
        [utf16(text, true).as_slice(), &[0x41]].concat(),
        [utf32(text, true).as_slice(), &[0, 0, 0x11, 0]].concat(),
        [utf32(text, false).as_slice(), &[0x41, 0]].concat(),
        [&[0xff, 0xfe], &utf16(text, false)[2..]].concat(),
        [&[0xff, 0xfe], bytes].concat(),
    ]
}

// MP-08/MP-11: every classification bypass reported on #903, each clean and
// in every malformed variant, plus the ordinary inputs that must stay data.
// `true` = copies as ordinary data; `false` = Vault-routed (or refused).
pub(crate) fn classification_cases() -> Vec<(&'static str, Vec<u8>, bool)> {
    cases(true)
}

// MP-08/MP-11: the four boundary paths run every bypass input and every
// malformed variant kind (one variant per input, rotating), not the full
// cross product, which `mp08_mp11_classification_table_fails_closed` covers.
pub(crate) fn boundary_cases() -> Vec<(&'static str, Vec<u8>, bool)> {
    cases(false)
}

fn cases(full: bool) -> Vec<(&'static str, Vec<u8>, bool)> {
    let variants = |index: usize, text: &str| {
        let mut all = malformed(text);
        if !full {
            all = vec![all.swap_remove(index % all.len())];
        }
        all
    };
    let escaped = r#"{"api\u005fkey":"synthetic-canary"}"#;
    let credentials: &[(&str, &str)] = &[
        // Wide text with a control character (#903 @39df80b71).
        ("bootstrap.ps1", "$env:API_KEY = 'synthetic-canary'\n"),
        // Escaped JSON keys in undecodable text (#903 @4e2992ec2).
        ("settings.json", escaped),
        (
            "settings.json",
            r#"{"vault\u005ffile_base64":"c3ludGhldGljLWNhbmFyeQ=="}"#,
        ),
        (
            "settings.json",
            r#"{"sealed\u005funlock_key":"c3ludGhldGljLWNhbmFyeQ=="}"#,
        ),
        ("settings.json", r#"{"ciphertext":"AAAA","kdf":"argon2id"}"#),
        // Decodable JSON that fails to parse (#903 @07aca9bc7).
        ("settings.json", r#"{"api\u005fkey":"synthetic-canary"}x"#),
        ("settings.json", r#"{"api\u005fkey":"synthetic-canary""#),
        ("settings.json", r#"{"api\u005fkey":"#),
        (
            "settings.json",
            r#"{"note":"a"} {"api\u005fkey":"synthetic-canary"}"#,
        ),
        // The same escaped key outside a JSON path, in data and in source.
        ("settings.txt", r#"{"api\u005fkey":"synthetic-canary"}x"#),
        ("config", r#"{"api\u005fkey":"synthetic-canary"}x"#),
        (
            "app.js",
            "const config = {\"api\\u005fkey\": \"synthetic-canary\"};\n",
        ),
        // Duplicate JSON keys (an earlier #903 review), plain and as JSONC.
        (
            "settings.json",
            r#"{"api_key":"synthetic-canary","api_key":""}"#,
        ),
        (
            "tsconfig.json",
            "{\"api_key\":\"synthetic-canary\", // c\n\"api_key\":\"\",}",
        ),
        // JSONC comments are free text; unterminated or JSON5 syntax is unclassified.
        (
            "tsconfig.json",
            "{\n  // API_KEY=synthetic-canary\n  \"strict\": true\n}\n",
        ),
        ("tsconfig.json", "{\"strict\": true} /* unterminated"),
        ("settings.json", "{strict: true}"),
    ];
    let ordinary: &[(&str, &str)] = &[
        ("tsconfig.json", "{\n  // Visit https://aka.ms/tsconfig\n  \"compilerOptions\": {\n    /* \"strict\": false, */\n    \"strict\": true,\n    \"paths\": {\"@/*\": [\"src/*\"]},\n  },\n}\n"),
        ("settings.json", r#"{"name":"caf\u00e9","max_tokens":4096}"#),
        ("settings.json", " \n"),
        ("README.md", "[![ci](https://example.test/badge.svg)](https://example.test)\nSay \"hello\": it works.\n"),
        ("bootstrap.ps1", "\u{feff}Write-Output 'ready'\n"),
        ("bootstrap.sh", "echo 'ordinary Unicode: \u{1f980}'\n"),
        ("main.c", "int main(void) { return 0; }\n\x0c\n/* page two */\n"),
        ("app.js", "const config = {\"name\": \"caf\\u00e9\"};\n"),
        ("app.js", "const status = ok ? \"PASS\" : \"FAIL\";\n"),
    ];
    let mut cases = Vec::new();
    for (index, (path, text)) in credentials.iter().enumerate() {
        cases.push((*path, text.as_bytes().to_vec(), false));
        cases.extend(
            variants(index, text)
                .into_iter()
                .map(|bytes| (*path, bytes, false)),
        );
    }
    for (index, (path, text)) in ordinary
        .iter()
        .copied()
        .chain(
            PACKAGE_METADATA
                .iter()
                .map(|(path, bytes)| (*path, std::str::from_utf8(bytes).unwrap())),
        )
        .enumerate()
    {
        cases.push((path, text.as_bytes().to_vec(), true));
        // Malformed text is unclassifiable even when its content is ordinary.
        cases.extend(
            variants(credentials.len() + index, text)
                .into_iter()
                .map(|bytes| (path, bytes, false)),
        );
    }
    // Declared wide text is never classified, even when its content is ordinary.
    for text in [
        "$env:API_KEY = 'synthetic-canary'\n",
        "Write-Output 'ready'\n",
    ] {
        for little_endian in [true, false] {
            cases.push(("bootstrap.ps1", utf16(text, little_endian), false));
            cases.push(("bootstrap.ps1", utf32(text, little_endian), false));
        }
    }
    // Opaque assets: an allow-listed extension and signature, not text.
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\xff\x00".to_vec();
    cases.extend([
        ("artwork.png", png.clone(), true),
        (
            "photo.JPG",
            b"\xff\xd8\xff\xe0\0\x10JFIF\0\xff".to_vec(),
            true,
        ),
        ("font.woff2", b"wOF2\0\x01\0\0\xff".to_vec(), true),
        (
            "artwork.png",
            [png.as_slice(), b"tEXt\0caf\xc3\xa9 \"ok\"".as_slice()].concat(),
            true,
        ),
        // MP-08/MP-11: dynamic shell names follow the same text policy in assets.
        (
            "artwork.png",
            [png.as_slice(), b"\0x$'abcdefgh\0a$b=cdefgh\0"].concat(),
            false,
        ),
        ("settings.json", png.clone(), false),
        ("photo.jpg", png.clone(), false),
        ("artwork.png", b"\x89PNG\0\xff".to_vec(), false),
        ("artwork.bin", b"\xff\0\x89".to_vec(), false),
        (
            "artwork.png",
            [b"\xef\xbb\xbf", png.as_slice()].concat(),
            false,
        ),
        (
            "artwork.png",
            [
                png.as_slice(),
                b"\0ghp_000000000000000000000000000000000000\0",
            ]
            .concat(),
            false,
        ),
        (
            "artwork.png",
            [png.as_slice(), escaped.as_bytes(), b"\xff"].concat(),
            false,
        ),
        (
            "artwork.png",
            [png.as_slice(), b"\0API_KEY=synthetic-canary\0"].concat(),
            false,
        ),
    ]);
    // MP-08/MP-11 (#903 r20 P1): every candidate in a printable asset
    // run must be inspected, even after an ordinary assignment or punctuation.
    for (path, signature) in [
        ("artwork.png", png.as_slice()),
        ("font.woff2", b"wOF2\0\x01\0\0\xff".as_slice()),
    ] {
        for run in [
            "FOO=bar API_KEY=synthetic-canary",
            "FOO=bar;API_KEY=synthetic-canary",
            "FOO=bar,API_KEY:synthetic-canary",
            "FOO=bar AUTH_TOKEN=synthetic-canary PASSWORD=synthetic-canary",
        ] {
            cases.push((
                path,
                [signature, b"\0", run.as_bytes(), b"\0"].concat(),
                false,
            ));
        }
    }
    // MP-08/MP-11 r21: source overlay/history, hostile target and packaged
    // Extension admission all exercise command credentials in both asset types.
    for (path, signature) in ASSET_SIGNATURES {
        for run in [
            "curl --user owner:synthetic-canary https://example.test",
            "curl -u owner:synthetic-canary https://example.test",
            "curl --proxy-user owner:synthetic-canary https://example.test",
            "wget --http-user owner:synthetic-canary https://example.test",
        ] {
            cases.push((
                *path,
                [*signature, b"\0", run.as_bytes(), b"\0"].concat(),
                false,
            ));
        }
    }
    cases
}

pub(crate) const PACKAGE_METADATA: &[(&str, &[u8])] = &[
    ("package.json", br#"{"dependencies":{"js-tokens":"^4.0.0","secret-tool":"^1.0.0"},"devDependencies":{"@example/tokenizer":"^2.0.0"},"scripts":{"build":"node build.js"}}"#),
    ("package-lock.json", br#"{"lockfileVersion":3,"packages":{"":{"dependencies":{"js-tokens":"^4.0.0"}},"node_modules/js-tokens":{"version":"4.0.0","resolved":"https://registry.npmjs.org/js-tokens/-/js-tokens-4.0.0.tgz","integrity":"sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="}},"dependencies":{"js-tokens":{"version":"4.0.0"}}}"#),
    ("usage.json", br#"{"input_tokens":42,"output_tokens":7,"total_tokens":49,"token_count":49,"max_tokens":4096}"#),
];

#[test]
fn mp08_mp11_target_classification_table_in_overlay_and_history() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    for history in [false, true] {
        for (path, bytes, data) in boundary_cases() {
            eprintln!("MP-08/MP-11 case {path} history={history} {bytes:?}");
            InlineShellFixture::file(path, &bytes, history).assert_target_result(data);
        }
    }
}

// MP-08/MP-11: one blob under two names is classified under both policies.
// `package.json` allows a dependency named `token`; `settings.json` does not.
#[test]
fn mp08_mp11_history_blob_is_classified_under_every_name() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    let bytes = br#"{"dependencies":{"token":"synthetic-canary"}}"#;
    assert!(validate_bytes("package.json", bytes).is_ok());
    assert!(validate_bytes("settings.json", bytes).is_err());
    let fixture = InlineShellFixture::file("settings.json", bytes, true);
    // HEAD names the same blob `package.json`, the name `rev-list` reports.
    fs::write(fixture.project.join("package.json"), bytes).unwrap();
    git(&fixture.project, &["add", "package.json"], MAX_FILE).unwrap();
    git(&fixture.project, &["commit", "-m", "alias"], MAX_FILE).unwrap();
    let items = scan_repository(&fixture.project).unwrap();
    assert!(
        items
            .iter()
            .any(|item| item.path.ends_with("/settings.json")),
        "{items:?}"
    );
    fixture.assert_target_refuses();
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
fn mp08_mp11_classification_table_fails_closed() {
    let wrong = classification_cases()
        .into_iter()
        .filter(|(path, bytes, data)| validate_bytes(path, bytes).is_ok() != *data)
        .map(|(path, bytes, data)| format!("{path} data={data} {bytes:?}"))
        .collect::<Vec<_>>();
    assert!(
        wrong.is_empty(),
        "MP-08/MP-11 {} misclassified:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

fn metadata_authentication_cases() -> Vec<(&'static str, &'static [u8])> {
    [
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
        (
            "settings.json",
            br#"{"api_key":"synthetic-canary","api_key":""}"#,
        ),
        (
            "settings.json",
            br#"{"nested":[{"token":"synthetic-canary","token":null}]}"#,
        ),
        (
            "package.json",
            br#"{"dependencies":{"js-tokens":"^4.0.0","js-tokens":"^5.0.0"}}"#,
        ),
    ]
    .to_vec()
}

#[test]
fn mp11_metadata_roles_do_not_hide_authentication_values() {
    for (path, bytes) in metadata_authentication_cases() {
        assert!(
            validate_bytes(path, bytes).is_err(),
            "MP-11 authentication fixture {path}"
        );
    }
}

// MP-08 / MP-11: exercise both boundaries independently for overlays and Git.
macro_rules! duplicate_json_boundary_test {
    ($name:ident, $history:expr, $assertion:ident) => {
        #[test]
        fn $name() {
            crate::test_support::isolated_env_test!();
            let _lock = crate::env_lock::lock();
            InlineShellFixture::file(
                "settings.json",
                br#"{"api_key":"synthetic-canary","api_key":""}"#,
                $history,
            )
            .$assertion();
        }
    };
}
duplicate_json_boundary_test!(
    r4_duplicate_json_overlay_source,
    false,
    assert_source_refuses
);
duplicate_json_boundary_test!(
    r4_duplicate_json_overlay_target,
    false,
    assert_target_refuses
);
duplicate_json_boundary_test!(
    r4_duplicate_json_history_source,
    true,
    assert_source_refuses
);
duplicate_json_boundary_test!(
    r4_duplicate_json_history_target,
    true,
    assert_target_refuses
);

#[test]
fn source_refuses_renamed_and_shared_blocked_historical_paths() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    for blocked in [".env", "auth.json"] {
        for rename in [true, false] {
            InlineShellFixture::historical_path(blocked, rename).assert_source_refuses();
        }
    }
}

#[test]
fn target_refuses_renamed_and_shared_blocked_historical_paths() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    for blocked in [".env", "auth.json"] {
        for rename in [true, false] {
            InlineShellFixture::historical_path(blocked, rename).assert_target_refuses();
        }
    }
}

#[test]
fn target_accepts_renamed_and_shared_ordinary_historical_paths() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    for path in ["notes.txt", "public\tname.txt"] {
        for rename in [true, false] {
            InlineShellFixture::historical_path(path, rename).assert_target_result(true);
        }
    }
}

#[test]
fn r2_source_scans_secret_objects_under_extra_bundle_refs() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    for (reference, hidden) in [
        ("refs/stash", false),
        ("refs/arbitrary/hidden", false),
        ("refs/stash", true),
    ] {
        let fixture = InlineShellFixture::file("README.txt", b"ordinary project\n", false);
        let archive = fixture.export_archive(Some((reference, true, hidden)));
        assert!(
            validate_development_archive(&archive.archive_path, "project", None).is_err(),
            "all imported pack objects must be inspected"
        );
    }
}

#[test]
fn r2_target_refuses_secret_objects_under_extra_bundle_refs() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    for (reference, hidden) in [
        ("refs/stash", false),
        ("refs/arbitrary/hidden", false),
        ("refs/stash", true),
    ] {
        InlineShellFixture::file("README.txt", b"ordinary project\n", false)
            .assert_target_result_with_extra_ref(false, Some((reference, true, hidden)));
    }
}

#[test]
fn r2_development_import_refuses_even_benign_extra_bundle_refs() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    use crate::managed_context::development::*;
    for reference in ["refs/stash", "refs/arbitrary/hidden"] {
        let fixture = InlineShellFixture::file("README.txt", b"ordinary project\n", false);
        let archive = fixture.export_archive(Some((reference, false, false)));
        let destination = fixture.root.join("plain-target");
        let error = import_development_context(DevelopmentContextImportRequest {
            archive_path: archive.archive_path,
            expected_archive_sha256: archive.archive_sha256,
            expected_project_id: "project".into(),
            expected_source_repositories: None,
            destination_root: destination.clone(),
        })
        .expect_err("importer must reject extra advertised refs before publication");
        assert!(error.to_string().contains("unexpected refs"));
        assert!(!destination.exists());
    }
}

#[test]
fn r3_guard_scans_every_gzip_member_the_importer_accepts() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    let fixture = InlineShellFixture::new("API_KEY=synthetic-canary\n", false);
    let archive = fixture.two_member_archive();
    assert!(
        validate_development_archive(&archive.archive_path, "project", None).is_err(),
        "source must inspect the second gzip member"
    );
    fixture.assert_target_archive(false, &archive);
    // Multi-member archives stay importable; refusal is content-driven.
    let ordinary = InlineShellFixture::new("echo ordinary\n", false);
    let archive = ordinary.two_member_archive();
    validate_development_archive(&archive.archive_path, "project", None).unwrap();
    ordinary.assert_target_archive(true, &archive);
}

// MP-08/MP-11: public-origin clean prose/source and every appended control.
fn public_text_cases() -> Vec<(&'static str, Vec<u8>, bool)> {
    let declarations = include_bytes!("fixtures/slugify-index.d.ts.txt");
    let prose = include_bytes!("fixtures/requests-commit-message.txt");
    let mut cases = vec![
        ("index.d.ts", declarations.to_vec(), true),
        ("", prose.to_vec(), true),
    ];
    for secret in [
        b"API_KEY=synthetic-live-secret-canary".as_slice(),
        b"-----BEGIN PRIVATE KEY-----\nsynthetic-private-material\n-----END PRIVATE KEY-----",
        b"env API_${NAME}=synthetic-canary worker",
        b"sh -c 'API_`printf KEY`=synthetic-canary worker'",
    ] {
        cases.push((
            "index.d.ts",
            [declarations.as_slice(), b"\n", secret].concat(),
            false,
        ));
    }
    cases.extend([
        (
            "",
            [
                prose.as_slice(),
                b"\nAPI_KEY=synthetic-real-secret-format-control\n",
            ]
            .concat(),
            false,
        ),
        ("", b"`API_KEY=synthetic-canary`".to_vec(), false),
        (
            "",
            b"env API_`printf KEY`=synthetic-canary worker".to_vec(),
            false,
        ),
    ]);
    cases
}

#[test]
fn mp08_mp11_public_source_retains_secret_detection() {
    for (path, bytes, data) in public_text_cases() {
        assert_eq!(
            validate_bytes(path, &bytes).is_ok(),
            data,
            "MP-08/MP-11 public fixture {path}"
        );
    }
}

#[cfg(unix)]
#[test]
fn mp08_mp11_admission_git_signal_retains_copy_retry() {
    crate::test_support::isolated_env_test!();
    let root = std::env::current_dir().unwrap();
    // Use the same owned-child seam on Linux and macOS; no /proc or shell kill.
    INTERRUPT_SCAN.with(|fault| fault.set(Some("status")));
    let error = git(&root, &["status", "--porcelain"], MAX_FILE).unwrap_err();
    INTERRUPT_SCAN.with(|fault| assert!(fault.get().is_none()));
    assert!(matches!(
        error,
        DaemonError::ManagedContext {
            code: "managed_context_unavailable",
            retryable: true,
            ..
        }
    ));
}

thread_local! {
    static INTERRUPT_SCAN: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}
// MP-11: supplementary interruption injection into a real Git child; never
// available through production configuration, serialized requests or PATH.
pub(super) fn interrupt_scan_child(child: &mut std::process::Child, args: &[&str]) {
    #[cfg(unix)]
    if args.contains(&"--batch") {
        BLOCK_OBJECT_READ.with(|object| {
            if let Some(oid) = object.borrow_mut().take() {
                use std::io::Write;
                use std::os::fd::AsRawFd;
                writeln!(child.stdin.as_mut().unwrap(), "{oid}").unwrap();
                let fd = child.stdout.as_ref().unwrap().as_raw_fd();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                loop {
                    let mut available: libc::c_int = 0;
                    assert_eq!(
                        unsafe { libc::ioctl(fd, libc::FIONREAD, &mut available) },
                        0
                    );
                    if available > 256 {
                        break;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "Git must publish an object header and partial body"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                assert!(child.id() > 1 && child.id() <= i32::MAX as u32);
                assert_eq!(
                    unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGSTOP) },
                    0
                );
            }
        });
    }
    INTERRUPT_SCAN.with(|fault| {
        if fault.get().is_some_and(|command| args.contains(&command)) {
            fault.set(None);
            assert!(child.id() > 1);
            child
                .kill()
                .expect("interrupt only the scan child this invocation owns");
        }
    });
}
#[test]
fn mp08_mp11_owner_package_admission_interruption_preserves_retry_and_reimports() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    for command in ["clone", "--batch"] {
        let fixture = InlineShellFixture::file("README.txt", b"ordinary project\n", true);
        let archive = fixture.export_archive(None);
        INTERRUPT_SCAN.with(|fault| fault.set(Some(command)));
        let error = fixture.target_archive_result(&archive).unwrap_err();
        assert!(
            matches!(
                error,
                DaemonError::ManagedContext {
                    code: "managed_context_unavailable",
                    retryable: true,
                    ..
                }
            ),
            "{command}: {error}"
        );
        assert!(!fixture.root.join("target").exists());
        INTERRUPT_SCAN.with(|fault| {
            assert!(
                fault.get().is_none(),
                "fault must reach the real admission scan"
            )
        });
        fs::remove_file(fixture.root.join("context.pkg")).unwrap();
        fixture.assert_target_archive(true, &archive);
    }
}
#[cfg(unix)]
#[test]
fn mp08_mp11_scan_limit_kill_stays_terminal() {
    let root = std::env::current_dir().unwrap();
    let error = git(&root, &["ls-files"], 1).unwrap_err();
    assert!(matches!(
        error,
        DaemonError::ManagedContext {
            retryable: false,
            ..
        }
    ));
    use std::os::unix::process::ExitStatusExt;
    assert!(scan_git_status(std::process::ExitStatus::from_raw(9), true).is_ok());
    assert!(scan_git_status(std::process::ExitStatus::from_raw(15), true).is_err());
}

// MP-08 / MP-11: exercise the real batch process, not only status classification.
#[test]
fn mp08_mp11_repository_history_limit_remains_terminal() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    let fixture =
        InlineShellFixture::file("oversized.txt", &vec![b'x'; MAX_FILE as usize + 1], true);
    for _ in 0..32 {
        let error = history::scan_repository(&fixture.project).unwrap_err();
        assert!(
            matches!(
                error,
                DaemonError::ManagedContext {
                    retryable: false,
                    ..
                }
            ),
            "{error}"
        );
        assert!(
            error.to_string().contains("16 MiB per-file limit"),
            "{error}"
        );
    }
}

thread_local! {
    static HISTORY_LIMIT: std::cell::Cell<std::time::Duration> = const { std::cell::Cell::new(std::time::Duration::from_secs(120)) };
    static BLOCK_OBJECT_READ: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}
pub(super) fn history_scan_limit() -> std::time::Duration {
    HISTORY_LIMIT.with(|limit| limit.get())
}

#[cfg(unix)]
#[test]
fn mp08_mp11_history_watchdog_during_object_read_reports_timeout() {
    let fixture = InlineShellFixture::file("large.txt", &vec![b'x'; 1024 * 1024], true);
    let oid = git(
        &fixture.project,
        &["rev-parse", "HEAD~1:large.txt"],
        MAX_FILE,
    )
    .unwrap();
    BLOCK_OBJECT_READ.with(|object| {
        *object.borrow_mut() = Some(String::from_utf8(oid).unwrap().trim().to_owned())
    });
    HISTORY_LIMIT.with(|limit| limit.set(std::time::Duration::from_millis(50)));
    let result = scan_repository(&fixture.project);
    HISTORY_LIMIT.with(|limit| limit.set(std::time::Duration::from_secs(120)));
    BLOCK_OBJECT_READ.with(|object| assert!(object.borrow().is_none()));
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("120 second time limit"),
        "{error}"
    );
    assert!(!error.to_string().contains("credential-bearing"), "{error}");
}
