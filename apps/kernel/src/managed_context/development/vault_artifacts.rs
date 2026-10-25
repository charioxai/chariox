//! MP-08 / MP-10 / MP-11: selected files/history use the owner's Vault at both
//! ends, with context-, identity- and artifact-bound kernel-to-kernel encryption.
use super::*;
use crate::config::{CredentialVaultBackend, DaemonConfig};
use crate::transport::relay_crypto;
use base64::Engine;
use std::cell::RefCell;
use zeroize::Zeroizing;

const PURPOSE: &[u8] = b"chariox-context-vault-artifact-v1";
const SERVICE: &str = "chariox-context-artifacts";

/// Value-free inventory for the existing human source review. The exporter
/// repeats classification on its exact immutable/bounded snapshot before seal.
pub(crate) fn review_items(
    roots: &BTreeMap<String, PathBuf>,
) -> Result<Vec<VaultItem>, DaemonError> {
    let mut found = BTreeSet::new();
    for (workspace, root) in roots {
        if fs::symlink_metadata(root.join(".git")).is_ok() {
            for mut item in super::super::credential_free::scan_repository(root)? {
                item.path = format!("{workspace}/{}", item.path);
                found.insert(item);
            }
        }
        if fs::symlink_metadata(root.join(".git")).is_ok() {
            let (entries, _, pending) = overlay::collect_overlay(
                root,
                "review",
                root,
                &mut ManifestMemoryBudget::new(),
                true,
            )?;
            for (object, bytes) in pending {
                for mut item in overlay::items_for_object(&object, &bytes, &entries) {
                    item.path = format!("{workspace}/{}", item.path);
                    found.insert(item);
                }
            }
            continue;
        }
        let patterns = overlay::read_context_ignore_patterns(root)?;
        let mut pending = vec![PathBuf::new()];
        let started = std::time::Instant::now();
        while let Some(relative) = pending.pop() {
            for entry in fs::read_dir(root.join(&relative))
                .map_err(|_| context_error("Vault review enumeration failed"))?
            {
                let entry = entry.map_err(|_| context_error("Vault review enumeration failed"))?;
                let path = relative.join(entry.file_name());
                let path = path
                    .to_str()
                    .ok_or_else(|| context_error("Vault review path is not UTF-8"))?;
                if overlay::context_force_excluded_path(path) {
                    continue;
                }
                if started.elapsed() > std::time::Duration::from_secs(120) {
                    return Err(context_error(
                        "Vault review exceeded the 120 second time limit",
                    ));
                }
                let metadata = fs::symlink_metadata(root.join(path))
                    .map_err(|_| context_error("Vault review metadata unavailable"))?;
                let rule_path = if metadata.is_dir() {
                    format!("{path}/")
                } else {
                    path.to_string()
                };
                if crate::workspace_live_sync_ignore::user_rules_exclude_path(&rule_path, &patterns)
                    && (!metadata.is_dir() || !overlay::may_include_descendant(&patterns, path))
                {
                    continue;
                }
                if metadata.file_type().is_symlink() {
                    continue;
                }
                if metadata.is_dir() {
                    pending.push(PathBuf::from(path));
                } else if metadata.is_file() {
                    let (bytes, _) = overlay::read_regular_file_without_following_symlinks(
                        root,
                        path,
                        MAX_OVERLAY_FILE_BYTES,
                    )?;
                    let bytes = Zeroizing::new(bytes);
                    if super::super::credential_free::validate_bytes(path, &bytes).is_err() {
                        found.insert(VaultItem {
                            path: format!("{workspace}/{path}"),
                            kind: "workspace file".into(),
                        });
                    }
                }
            }
        }
    }
    Ok(found.into_iter().collect())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultItem {
    pub path: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultArtifactBinding {
    pub context_id: String,
    pub source_kernel_id: String,
    pub target_kernel_id: String,
    pub project_id: String,
    pub object_path: String,
    pub plaintext_sha256: String,
    pub plaintext_size_bytes: u64,
    pub items: Vec<VaultItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VaultArtifact {
    pub binding: VaultArtifactBinding,
    pub encrypted_sha256: String,
    pub encrypted_size_bytes: u64,
}

pub(crate) struct ExportVault<'a> {
    pub config: &'a DaemonConfig,
    pub context_id: &'a str,
    pub target_kernel_id: &'a str,
    pub target_public_key: &'a str,
    pub project_id: &'a str,
    artifacts: RefCell<BTreeMap<String, VaultArtifact>>,
}

impl<'a> ExportVault<'a> {
    pub fn new(
        config: &'a DaemonConfig,
        context_id: &'a str,
        target_kernel_id: &'a str,
        target_public_key: &'a str,
        project_id: &'a str,
    ) -> Self {
        Self {
            config,
            context_id,
            target_kernel_id,
            target_public_key,
            project_id,
            artifacts: RefCell::new(BTreeMap::new()),
        }
    }

    pub fn artifacts(&self) -> Vec<VaultArtifact> {
        self.artifacts.borrow().values().cloned().collect()
    }

    /// No plaintext is written to source staging. The durable source record is
    /// created first; encryption reads back that record through the Vault API.
    pub fn stage(
        &self,
        staging: &Path,
        object_path: &str,
        bytes: &[u8],
        mut items: Vec<VaultItem>,
    ) -> Result<(), DaemonError> {
        validate_relative_path(object_path)?;
        // Sealed artifacts are base64 inside JSON, so admit their encoded size against the volume.
        let needed = if items.is_empty() {
            bytes.len() as u64
        } else {
            (bytes.len() as u64).saturating_mul(3) / 2 + 4096
        };
        let room = crate::managed_context::limits::disk_budget(staging);
        if needed > room {
            return Err(context_error(
                crate::managed_context::limits::disk_shortage(
                    "repository artifact staging",
                    staging,
                    needed,
                    room,
                ),
            ));
        }
        let destination = staging.join(object_path);
        if let Some(parent) = destination.parent() {
            create_private_directory(parent)?;
        }
        if items.is_empty() {
            return write_private_file(&destination, bytes);
        }
        items.sort();
        items.dedup();
        require_encrypted_vault(self.config)?;
        let vault = crate::secret::project_environment_vault(self.config)?;
        let digest = sha256_bytes(bytes);
        let value = Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(bytes));
        vault.set_secret(SERVICE, &digest, &value)?;
        let stored = Zeroizing::new(vault.get_secret(SERVICE, &digest)?);
        let restored = Zeroizing::new(
            base64::engine::general_purpose::STANDARD
                .decode(stored.as_bytes())
                .map_err(|_| context_error("source Vault artifact could not be restored"))?,
        );
        if restored.as_slice() != bytes {
            return Err(context_error("source Vault artifact verification failed"));
        }
        let binding = VaultArtifactBinding {
            context_id: self.context_id.into(),
            source_kernel_id: self.config.daemon_id.clone(),
            target_kernel_id: self.target_kernel_id.into(),
            project_id: self.project_id.into(),
            object_path: object_path.into(),
            plaintext_sha256: digest,
            plaintext_size_bytes: bytes.len() as u64,
            items,
        };
        let aad = serde_json::to_vec(&binding)
            .map_err(|_| context_error("Vault binding could not be encoded"))?;
        let encrypted = relay_crypto::encrypt_payload_for_peer_bound(
            &self.config.relay_private_key,
            self.target_public_key,
            PURPOSE,
            &aad,
            &restored,
        )?;
        let encoded = serde_json::to_vec(&encrypted)
            .map_err(|_| context_error("Vault artifact could not be encoded"))?;
        write_private_file(&destination, &encoded)?;
        crate::logging::info_with_fields(
            "context_vault",
            "Copy artifact recorded in source Vault and sealed for target",
            serde_json::json!({
                "context_id": self.context_id, "artifact": object_path, "items": binding.items, "vault_verified": true,
            }),
        );
        self.artifacts.borrow_mut().insert(
            object_path.into(),
            VaultArtifact {
                binding,
                encrypted_sha256: sha256_bytes(&encoded),
                encrypted_size_bytes: encoded.len() as u64,
            },
        );
        Ok(())
    }
}

fn require_encrypted_vault(config: &DaemonConfig) -> Result<(), DaemonError> {
    if config.user_config.credential_vault.backend != CredentialVaultBackend::CharioxEncrypted {
        return Err(context_error(
            "secret-bearing Copy requires the owner's encrypted Chariox Vault",
        ));
    }
    Ok(())
}

/// Restore only within the staged target workspace. Plain artifacts are linked
/// into this workspace-local view; secret artifacts are authenticated, recorded
/// in the target Vault, read back, verified and then materialized there.
pub(super) fn restore(
    manifest: &DevelopmentContextManifest,
    artifacts: &Path,
    workspace: &Path,
    authority: Option<&crate::project_environment::ProjectEnvironmentImportAuthority>,
) -> Result<PathBuf, DaemonError> {
    if manifest.vault_artifacts.is_empty() {
        return Ok(artifacts.to_path_buf());
    }
    let authority = authority
        .ok_or_else(|| context_error("Vault artifacts require authenticated target authority"))?;
    require_encrypted_vault(&authority.config)?;
    let vault = crate::secret::project_environment_vault(&authority.config)?;
    let restored_root = workspace.join(".vault-transfer-artifacts");
    create_private_directory(&restored_root)?;
    link_plain_artifacts(
        artifacts,
        artifacts,
        &restored_root,
        &manifest.vault_artifacts,
    )?;
    for artifact in &manifest.vault_artifacts {
        let binding = &artifact.binding;
        if binding.context_id != authority.context_id
            || binding.source_kernel_id != authority.source_kernel_id
            || binding.target_kernel_id != authority.target_kernel_id
            || binding.project_id != manifest.project_id
        {
            return Err(context_error(
                "Vault artifact source, target or context mismatch",
            ));
        }
        let (encoded, _) = overlay::read_regular_file_without_following_symlinks(
            artifacts,
            &binding.object_path,
            crate::managed_context::limits::memory_budget(),
        )?;
        if encoded.len() as u64 != artifact.encrypted_size_bytes
            || sha256_bytes(&encoded) != artifact.encrypted_sha256
        {
            return Err(context_error("encrypted Vault artifact digest mismatch"));
        }
        let payload: chariox_relay::protocol::EncryptedRelayPayload =
            serde_json::from_slice(&encoded)
                .map_err(|_| context_error("invalid encrypted Vault artifact"))?;
        if crate::runtime::terminal_pairings::public_key_thumbprint(&payload.sender_public_key)
            != authority.source_key_thumbprint
        {
            return Err(context_error("Vault artifact sender mismatch"));
        }
        relay_crypto::validate_encrypted_payload_shape(
            &payload,
            usize::try_from(crate::managed_context::limits::memory_budget())
                .unwrap_or(usize::MAX / 2)
                .saturating_mul(2),
        )?;
        let aad = serde_json::to_vec(binding)
            .map_err(|_| context_error("Vault binding could not be encoded"))?;
        let decrypted = relay_crypto::decrypt_payload_for_private_key_bound(
            &authority.config.relay_private_key,
            &payload,
            PURPOSE,
            &aad,
        )?;
        if decrypted.plaintext.len() as u64 != binding.plaintext_size_bytes
            || sha256_bytes(&decrypted.plaintext) != binding.plaintext_sha256
        {
            return Err(context_error(
                "Vault artifact plaintext verification failed",
            ));
        }
        let value =
            Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(&decrypted.plaintext));
        vault.set_secret(SERVICE, &binding.plaintext_sha256, &value)?;
        let stored = Zeroizing::new(vault.get_secret(SERVICE, &binding.plaintext_sha256)?);
        let restored = Zeroizing::new(
            base64::engine::general_purpose::STANDARD
                .decode(stored.as_bytes())
                .map_err(|_| context_error("target Vault artifact could not be restored"))?,
        );
        if sha256_bytes(&restored) != binding.plaintext_sha256 {
            return Err(context_error("target Vault artifact verification failed"));
        }
        let destination = restored_root.join(&binding.object_path);
        if let Some(parent) = destination.parent() {
            create_private_directory(parent)?;
        }
        write_private_file(&destination, &restored)?;
        crate::logging::info_with_fields(
            "context_vault",
            "Copy artifact restored through target Vault",
            serde_json::json!({
                "context_id": binding.context_id, "artifact": binding.object_path, "items": binding.items, "vault_verified": true,
            }),
        );
    }
    Ok(restored_root)
}

fn link_plain_artifacts(
    root: &Path,
    current: &Path,
    destination: &Path,
    sealed: &[VaultArtifact],
) -> Result<(), DaemonError> {
    for entry in fs::read_dir(current).map_err(|_| context_error("artifact enumeration failed"))? {
        let entry = entry.map_err(|_| context_error("artifact enumeration failed"))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| context_error("artifact escaped root"))?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| context_error("artifact metadata unavailable"))?;
        if metadata.is_dir() {
            create_private_directory(&destination.join(relative))?;
            link_plain_artifacts(root, &path, destination, sealed)?;
        } else if metadata.is_file()
            && !sealed
                .iter()
                .any(|s| Path::new(&s.binding.object_path) == relative)
        {
            fs::hard_link(&path, destination.join(relative))
                .map_err(|_| context_error("artifact workspace link failed"))?;
        } else if !metadata.is_file() {
            return Err(context_error("artifact is not a regular file"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn config(root: &Path, id: &str) -> DaemonConfig {
        let mut config = DaemonConfig::new(id, id, "owner");
        config.user_config.credential_vault.backend = CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.path = root
            .join(format!("{id}-vault.json"))
            .to_string_lossy()
            .into_owned();
        crate::secret::create_chariox_encrypted_vault_for_test(
            Path::new(&config.user_config.credential_vault.path),
            "supplementary-test-passphrase",
        )
        .unwrap();
        crate::secret::unlock_chariox_encrypted_vault(
            &config.user_config.credential_vault.path,
            "supplementary-test-passphrase",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        config
    }

    // A Git bundle or overlay object staged through the Vault must be admitted against the staging
    // volume, sealed or not, before anything is written there.
    #[test]
    fn staging_refuses_artifacts_that_do_not_fit_the_volume_before_writing() {
        use crate::managed_context::limits::{with_free_disk, DISK_RESERVE_BYTES};
        let root = std::env::temp_dir().join(format!("vault-room-{:032x}", rand::random::<u128>()));
        fs::create_dir_all(&root).unwrap();
        let _cleanup = Cleanup(root.clone());
        let source = config(&root, "source");
        let target = config(&root, "target");
        let vault = ExportVault::new(
            &source,
            "context",
            &target.daemon_id,
            &target.relay_public_key,
            "project",
        );
        let staging = root.join("staging");
        let bytes = vec![7_u8; 1 << 20];
        let sealed = || {
            vec![VaultItem {
                path: "workspace/.env".into(),
                kind: "workspace file".into(),
            }]
        };
        let free = DISK_RESERVE_BYTES + (512 << 10);
        for items in [Vec::new(), sealed()] {
            let error = with_free_disk(free, || vault.stage(&staging, "bundle", &bytes, items))
                .expect_err("1 MiB does not fit 512 KiB of room");
            assert!(
                error.to_string().contains("free space on that volume"),
                "{error}"
            );
            assert!(!staging.join("bundle").exists());
        }
        with_free_disk(DISK_RESERVE_BYTES + (4 << 20), || {
            vault.stage(&staging, "plain", &bytes, Vec::new())
        })
        .expect("fits");
    }

    // MP-08/MP-10/MP-11: many large secret-bearing blobs must be refused before
    // they replace the owner's Vault with a file its readers reject.
    #[test]
    fn mp08_mp11_oversized_artifacts_leave_owner_vault_untouched() {
        let root =
            std::env::temp_dir().join(format!("mp08-vault-bound-{:032x}", rand::random::<u128>()));
        fs::create_dir_all(&root).unwrap();
        let _cleanup = Cleanup(root.clone());
        let source = config(&root, "source");
        let target = config(&root, "target");
        let path = PathBuf::from(&source.user_config.credential_vault.path);
        let vault = ExportVault::new(
            &source,
            "context",
            &target.daemon_id,
            &target.relay_public_key,
            "project",
        );
        let staging = root.join("staging");
        let item = || {
            vec![VaultItem {
                path: "workspace/.env".into(),
                kind: "workspace file".into(),
            }]
        };
        let blob = |fill: u8| vec![fill; MAX_OVERLAY_FILE_BYTES as usize];
        vault.stage(&staging, "a", &blob(b'a'), item()).unwrap();
        vault.stage(&staging, "b", &blob(b'b'), item()).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(vault.stage(&staging, "c", &blob(b'c'), item()).is_err());
        assert!(
            fs::read(&path).unwrap() == before,
            "owner Vault was replaced"
        );
        let stored = Zeroizing::new(
            crate::secret::get_chariox_encrypted_vault_secret_for_test(
                &source.user_config.credential_vault.path,
                SERVICE,
                &sha256_bytes(&blob(b'a')),
            )
            .unwrap(),
        );
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(stored.as_bytes())
                .unwrap(),
            blob(b'a')
        );
        assert!(!staging.join("c").exists());
        assert_eq!(
            fs::read_dir(&root).unwrap().count(),
            3,
            "temp Vault left behind"
        );
        crate::secret::lock_chariox_encrypted_vault(&source.user_config.credential_vault.path)
            .unwrap();
        crate::secret::lock_chariox_encrypted_vault(&target.user_config.credential_vault.path)
            .unwrap();
    }

    #[test]
    fn mp08_mp11_vault_file_roundtrip_and_plain_archive_exclusion() {
        let root =
            std::env::temp_dir().join(format!("mp08-vault-copy-{:032x}", rand::random::<u128>()));
        fs::create_dir_all(&root).unwrap();
        let _cleanup = Cleanup(root.clone());
        let source = config(&root, "source");
        let target = config(&root, "target");
        let workspace = root.join("workspace");
        fs::create_dir(&workspace).unwrap();
        let secret = b"PASSWORD=supplementary-file-secret\n";
        fs::write(workspace.join(".env"), secret).unwrap();
        // A shared hash first seen through a benign alias must still be sealed.
        fs::write(workspace.join("alias.txt"), secret).unwrap();
        fs::write(workspace.join("artwork.png"), b"\x89PNG\r\n\x1a\n\0\xff").unwrap();
        let vault = ExportVault::new(
            &source,
            "context",
            &target.daemon_id,
            &target.relay_public_key,
            "project",
        );
        let exported = super::super::export::export_development_context_with_vault(
            DevelopmentContextExportRequest {
                project_id: "project".into(),
                repositories: vec![DevelopmentRepositorySelection {
                    workspace_id: "workspace".into(),
                    worktree_id: None,
                    worktree_path: workspace,
                    role: DevelopmentRepositoryRole::Primary,
                }],
                archive_path: root.join("copy.tar.gz"),
            },
            None,
            Some(&vault),
        )
        .unwrap();
        assert_eq!(
            exported.manifest.schema_version,
            VAULT_CONTEXT_SCHEMA_VERSION
        );
        assert_eq!(exported.manifest.vault_artifacts.len(), 1);
        let mut tar = tar::Archive::new(flate2::read::MultiGzDecoder::new(
            File::open(&exported.archive_path).unwrap(),
        ));
        for entry in tar.entries().unwrap() {
            let mut bytes = Vec::new();
            entry.unwrap().read_to_end(&mut bytes).unwrap();
            assert!(
                !bytes.windows(secret.len()).any(|window| window == secret),
                "secret escaped into plaintext archive"
            );
        }
        let source_entry = Zeroizing::new(
            crate::secret::get_chariox_encrypted_vault_secret_for_test(
                &source.user_config.credential_vault.path,
                SERVICE,
                &sha256_bytes(secret),
            )
            .unwrap(),
        );
        assert!(
            base64::engine::general_purpose::STANDARD
                .decode(source_entry.as_bytes())
                .unwrap()
                .as_slice()
                == secret
        );
        let authority = crate::project_environment::ProjectEnvironmentImportAuthority {
            config: target.clone(),
            context_id: "context".into(),
            source_kernel_id: source.daemon_id.clone(),
            source_key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(
                &source.relay_public_key,
            ),
            target_kernel_id: target.daemon_id.clone(),
        };
        let request = DevelopmentContextImportRequest {
            archive_path: exported.archive_path.clone(),
            expected_archive_sha256: exported.archive_sha256.clone(),
            expected_project_id: "project".into(),
            expected_source_repositories: None,
            destination_root: root.join("target-project"),
        };
        let mut wrong = authority.clone();
        wrong.context_id = "different-context".into();
        assert!(
            super::super::import::import_development_context_with_options(
                request.clone(),
                UNBOUNDED_BYTES,
                u64::MAX,
                None,
                Some(&wrong)
            )
            .is_err()
        );
        assert!(!request.destination_root.exists());
        let (imported, _) = super::super::import::import_development_context_with_options(
            request,
            UNBOUNDED_BYTES,
            u64::MAX,
            None,
            Some(&authority),
        )
        .unwrap();
        let restored = &imported.repositories[0].destination_path;
        assert!(fs::read(restored.join(".env")).unwrap().as_slice() == secret);
        assert!(fs::read(restored.join("alias.txt")).unwrap().as_slice() == secret);
        assert!(fs::read(restored.join("artwork.png")).unwrap() == b"\x89PNG\r\n\x1a\n\0\xff");
        assert!(!imported
            .destination_root
            .join(".vault-transfer-artifacts")
            .exists());
        crate::secret::lock_chariox_encrypted_vault(&source.user_config.credential_vault.path)
            .unwrap();
        crate::secret::lock_chariox_encrypted_vault(&target.user_config.credential_vault.path)
            .unwrap();
    }
}
