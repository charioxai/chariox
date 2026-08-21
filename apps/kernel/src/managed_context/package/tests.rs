use super::*;

use crate::managed_context::kernel::{
    KernelContextCompatibility, KernelContextPayload, KernelContextSnapshot,
};

#[test]
fn explicit_empty_package_applies_a_real_development_context_without_kernel_state() {
    let root = std::env::temp_dir().join(format!(
        "chariox-managed-context-empty-apply-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let repository = root.join("repository");
    fs::create_dir_all(&repository).expect("create source repository");
    git(&repository, &["init", "-b", "main"]);
    git(
        &repository,
        &["config", "user.email", "tests@chariox.local"],
    );
    git(&repository, &["config", "user.name", "Chariox Tests"]);
    fs::write(repository.join("tracked.txt"), "explicit Empty\n").expect("write source file");
    git(&repository, &["add", "tracked.txt"]);
    git(&repository, &["commit", "-m", "initial"]);
    let development = crate::managed_context::development::export_development_context(
        crate::managed_context::development::DevelopmentContextExportRequest {
            project_id: "project-empty-apply".to_string(),
            repositories: vec![
                crate::managed_context::development::DevelopmentRepositorySelection {
                    workspace_id: "workspace-primary".to_string(),
                    worktree_id: None,
                    worktree_path: repository,
                    role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                },
            ],
            archive_path: root.join("development.tar.gz"),
        },
    )
    .expect("export development context");
    let binding = ManagedContextPackageBinding {
        plan: ManagedContextPlanBinding {
            context_id: "context-empty-apply".to_string(),
            plan_digest: format!("sha256:{}", "1".repeat(64)),
            kernel_context: ManagedContextKernelSelection::Empty,
            development: ManagedContextDevelopmentSelection::SourceProject {
                project_id: "project-empty-apply".to_string(),
                repositories: vec![DevelopmentSourceRepositoryBinding {
                    role: DevelopmentRepositoryRole::Primary,
                    workspace_id: "workspace-primary".to_string(),
                    worktree_id: None,
                }],
            },
            provider_accounts: ManagedContextProviderAccountSelection::None,
            git_credentials: ManagedContextGitCredentialSelection::None,
        },
        target_environment_id: "environment-empty-apply".to_string(),
        source_kernel_id: "source-kernel-empty-apply".to_string(),
        source_key_thumbprint: "a".repeat(64),
        target_kernel_id: "target-kernel-empty-apply".to_string(),
        target_key_thumbprint: "b".repeat(64),
    };
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
        package_path: root.join("context.chariox"),
    })
    .expect("compose explicit Empty package");
    let receipt = apply_managed_context_package(ManagedContextPackageApplicationRequest {
        transfer_id: "ctx_empty_apply".to_string(),
        package_path: package.package_path,
        expected_package_sha256: package.package_sha256,
        expected_binding: binding,
        development_destination_root: root.join("managed/project"),
        target_private_key: "unused-for-explicit-empty".to_string(),
    })
    .expect("apply explicit Empty package");
    assert!(matches!(
        receipt.kernel_context,
        ManagedContextImportedKernelContext::Empty
    ));
    let ManagedContextImportedDevelopment::FromSource { receipt, .. } = receipt.development else {
        panic!("selected development context became Empty")
    };
    assert_eq!(
        fs::read_to_string(receipt.repositories[0].destination_path.join("tracked.txt"))
            .expect("read imported file"),
        "explicit Empty\n"
    );
    fs::remove_dir_all(root).expect("remove Empty apply fixture");
}

#[test]
fn empty_package_round_trips_as_explicit_empty_context() {
    let fixture = PackageFixture::new("empty");
    let exported =
        export_managed_context_package(fixture.export_request(ManagedContextPackageKernel::Empty))
            .expect("export explicit Empty package");
    assert!(exported.kernel_context_snapshot_sha256.is_none());
    let extracted = extract_managed_context_package(fixture.import_request(&exported))
        .expect("extract explicit Empty package");
    assert!(matches!(
        extracted.kernel_context,
        ManagedContextPackageKernel::Empty
    ));
    let ExtractedManagedContextDevelopment::FromSource { archive_path, .. } =
        &extracted.development
    else {
        panic!("selected development context became Empty")
    };
    assert_eq!(
        fs::read(archive_path).expect("read development component"),
        fixture.development_bytes
    );
    let component_root = extracted.component_root().to_path_buf();
    drop(extracted);
    assert!(!component_root.exists());
    fixture.cleanup();
}

#[test]
fn package_applies_without_a_development_component_when_the_plan_selects_empty() {
    let fixture = PackageFixture::new("no-development");
    let mut request = fixture.export_request(ManagedContextPackageKernel::Empty);
    request.plan.development = ManagedContextDevelopmentSelection::Empty;
    request.development = ManagedContextPackageDevelopment::Empty;
    let exported = export_managed_context_package(request).expect("export no-development package");
    assert!(exported.development_archive_sha256.is_none());
    let binding = ManagedContextPackageBinding {
        plan: exported.plan.clone(),
        ..fixture.binding.clone()
    };
    let receipt = apply_managed_context_package(ManagedContextPackageApplicationRequest {
        transfer_id: "ctx_no_development".to_string(),
        package_path: exported.package_path,
        expected_package_sha256: exported.package_sha256,
        expected_binding: binding,
        development_destination_root: fixture.root.join("unused-development"),
        target_private_key: "unused-for-empty-context".to_string(),
    })
    .expect("apply no-development package");
    assert!(matches!(
        receipt.development,
        ManagedContextImportedDevelopment::Empty
    ));
    assert!(!fixture.root.join("unused-development").exists());
    fixture.cleanup();
}

#[test]
fn package_manifest_carries_a_near_cloud_limit_plan() {
    let fixture = PackageFixture::new("large-plan");
    let mut request = fixture.export_request(ManagedContextPackageKernel::Empty);
    request.plan.development = ManagedContextDevelopmentSelection::SourceProject {
        project_id: "project-large-plan".to_string(),
        repositories: (0..16)
            .map(|index| DevelopmentSourceRepositoryBinding {
                role: if index == 0 {
                    DevelopmentRepositoryRole::Primary
                } else {
                    DevelopmentRepositoryRole::Supporting
                },
                workspace_id: format!("workspace-{index}-{}", "x".repeat(4_060)),
                worktree_id: None,
            })
            .collect(),
    };
    let plan_bytes = serde_json::to_vec(&request.plan).expect("serialize large plan");
    assert!(plan_bytes.len() > 63 * 1024);
    assert!(plan_bytes.len() <= MAX_PLAN_BINDING_BYTES);

    let exported = export_managed_context_package(request).expect("export near-limit plan");
    let package_bytes = fs::read(&exported.package_path).expect("read near-limit package");
    let manifest_length = u32::from_be_bytes(
        package_bytes[PACKAGE_MAGIC.len()..PACKAGE_MAGIC.len() + 4]
            .try_into()
            .expect("manifest length field"),
    ) as usize;
    assert!(manifest_length > 64 * 1024);
    assert!(manifest_length <= MAX_MANIFEST_BYTES);
    let extracted = extract_managed_context_package(fixture.import_request(&exported))
        .expect("extract near-limit plan");
    assert!(matches!(
        extracted.kernel_context,
        ManagedContextPackageKernel::Empty
    ));
    drop(extracted);
    fixture.cleanup();
}

#[test]
fn source_kernel_package_round_trips_with_identity_bindings() {
    let fixture = PackageFixture::new("source");
    let snapshot = fixture.kernel_snapshot();
    let exported = export_managed_context_package(
        fixture.export_request(ManagedContextPackageKernel::FromKernel(snapshot.clone())),
    )
    .expect("export source-kernel package");
    assert_eq!(
        exported.kernel_context_snapshot_sha256.as_deref(),
        Some(snapshot.snapshot_sha256.as_str())
    );
    let extracted = extract_managed_context_package(fixture.import_request(&exported))
        .expect("extract source-kernel package");
    match &extracted.kernel_context {
        ManagedContextPackageKernel::FromKernel(imported) => assert_eq!(imported, &snapshot),
        ManagedContextPackageKernel::Empty => panic!("source context silently became Empty"),
    }
    drop(extracted);
    fixture.cleanup();
}

#[test]
fn package_rejects_a_different_authenticated_binding() {
    let fixture = PackageFixture::new("binding");
    let exported =
        export_managed_context_package(fixture.export_request(ManagedContextPackageKernel::Empty))
            .expect("export package");
    let mut request = fixture.import_request(&exported);
    request.expected_binding.target_environment_id = "environment-other".to_string();
    let error = extract_managed_context_package(request).expect_err("binding mismatch must fail");
    assert!(matches!(
        error,
        DaemonError::ManagedContext {
            code: "invalid_managed_context",
            retryable: false,
            ..
        }
    ));
    fixture.cleanup();
}

#[test]
fn package_rejects_a_substituted_workspace_selection_before_publication() {
    let fixture = PackageFixture::new("workspace-binding");
    let exported =
        export_managed_context_package(fixture.export_request(ManagedContextPackageKernel::Empty))
            .expect("export package");
    let mut request = fixture.import_request(&exported);
    let ManagedContextDevelopmentSelection::SourceProject { repositories, .. } =
        &mut request.expected_binding.plan.development
    else {
        panic!("fixture must select source development")
    };
    repositories[0].workspace_id = "workspace-substituted".to_string();
    let error = extract_managed_context_package(request)
        .expect_err("substituted Workspace selection must fail");
    assert!(matches!(
        error,
        DaemonError::ManagedContext {
            code: "invalid_managed_context",
            retryable: false,
            ..
        }
    ));
    fixture.cleanup();
}

#[test]
fn package_rejects_mutation_after_outer_digest_is_fixed() {
    let fixture = PackageFixture::new("mutation");
    let exported =
        export_managed_context_package(fixture.export_request(ManagedContextPackageKernel::Empty))
            .expect("export package");
    let mut bytes = fs::read(&exported.package_path).expect("read package");
    let index = bytes.len() / 2;
    bytes[index] ^= 1;
    fs::write(&exported.package_path, bytes).expect("mutate package in place");
    let error = extract_managed_context_package(fixture.import_request(&exported))
        .expect_err("mutated package must fail");
    assert!(matches!(
        error,
        DaemonError::ManagedContext {
            code: "invalid_managed_context",
            retryable: false,
            ..
        }
    ));
    fixture.cleanup();
}

struct PackageFixture {
    root: PathBuf,
    development_path: PathBuf,
    development_bytes: Vec<u8>,
    binding: ManagedContextPackageBinding,
}

fn git(root: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

impl PackageFixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-context-package-{label}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir_all(&root).expect("create package fixture");
        let development_path = root.join("development.tar.gz");
        let development_bytes = b"bounded-development-archive".to_vec();
        fs::write(&development_path, &development_bytes).expect("write development fixture");
        Self {
            root,
            development_path,
            development_bytes,
            binding: ManagedContextPackageBinding {
                plan: ManagedContextPlanBinding {
                    context_id: "context-1".to_string(),
                    plan_digest: format!("sha256:{}", "1".repeat(64)),
                    kernel_context: ManagedContextKernelSelection::Empty,
                    development: ManagedContextDevelopmentSelection::SourceProject {
                        project_id: "project-1".to_string(),
                        repositories: vec![DevelopmentSourceRepositoryBinding {
                            role: DevelopmentRepositoryRole::Primary,
                            workspace_id: "workspace-1".to_string(),
                            worktree_id: None,
                        }],
                    },
                    provider_accounts: ManagedContextProviderAccountSelection::None,
                    git_credentials: ManagedContextGitCredentialSelection::None,
                },
                target_environment_id: "environment-1".to_string(),
                source_kernel_id: "source-kernel-1".to_string(),
                source_key_thumbprint: "a".repeat(64),
                target_kernel_id: "target-kernel-1".to_string(),
                target_key_thumbprint: "b".repeat(64),
            },
        }
    }

    fn export_request(
        &self,
        kernel_context: ManagedContextPackageKernel,
    ) -> ManagedContextPackageExportRequest {
        let mut plan = self.binding.plan.clone();
        plan.kernel_context = match &kernel_context {
            ManagedContextPackageKernel::Empty => ManagedContextKernelSelection::Empty,
            ManagedContextPackageKernel::FromKernel(_) => {
                ManagedContextKernelSelection::SourceKernel
            }
        };
        ManagedContextPackageExportRequest {
            plan,
            target_environment_id: self.binding.target_environment_id.clone(),
            source_kernel_id: self.binding.source_kernel_id.clone(),
            source_key_thumbprint: self.binding.source_key_thumbprint.clone(),
            target_kernel_id: self.binding.target_kernel_id.clone(),
            target_key_thumbprint: self.binding.target_key_thumbprint.clone(),
            development: ManagedContextPackageDevelopment::FromSource {
                archive_path: self.development_path.clone(),
                archive_sha256: sha256_bytes(&self.development_bytes),
            },
            kernel_context,
            package_path: self.root.join("context.chariox"),
        }
    }

    fn import_request(
        &self,
        exported: &ManagedContextPackageExportResult,
    ) -> ManagedContextPackageImportRequest {
        ManagedContextPackageImportRequest {
            package_path: exported.package_path.clone(),
            expected_package_sha256: exported.package_sha256.clone(),
            expected_binding: ManagedContextPackageBinding {
                plan: exported.plan.clone(),
                ..self.binding.clone()
            },
        }
    }

    fn kernel_snapshot(&self) -> KernelContextSnapshot {
        KernelContextSnapshot {
            payload: KernelContextPayload {
                schema_version: 1,
                context_id: self.binding.plan.context_id.clone(),
                source_kernel_id: self.binding.source_kernel_id.clone(),
                source_key_thumbprint: self.binding.source_key_thumbprint.clone(),
                target_kernel_id: self.binding.target_kernel_id.clone(),
                target_key_thumbprint: self.binding.target_key_thumbprint.clone(),
                compatibility: KernelContextCompatibility {
                    source_kernel_version: "0.1.0".to_string(),
                    local_daemon_protocol_version: crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
                    relay_peer_protocol_version:
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                },
                extensions: Vec::new(),
                dependencies: Vec::new(),
                vault: crate::secret::TransferredVaultSnapshot {
                    schema_version: 1,
                    context_id: self.binding.plan.context_id.clone(),
                    source_kernel_id: self.binding.source_kernel_id.clone(),
                    source_key_thumbprint: self.binding.source_key_thumbprint.clone(),
                    target_kernel_id: self.binding.target_kernel_id.clone(),
                    target_key_thumbprint: self.binding.target_key_thumbprint.clone(),
                    vault_sha256: "c".repeat(64),
                    vault_size_bytes: 1,
                    vault_file_base64: "AA==".to_string(),
                    sealed_unlock_key: chariox_relay::protocol::EncryptedRelayPayload {
                        sender_public_key: "sender".to_string(),
                        nonce: "nonce".to_string(),
                        ciphertext: "ciphertext".to_string(),
                    },
                },
            },
            snapshot_sha256: "d".repeat(64),
        }
    }

    fn cleanup(&self) {
        fs::remove_dir_all(&self.root).expect("remove package fixture");
    }
}
