//! Private, credential-free generations issued to a particular receiving placement.
//! Keep unacknowledged requests across kernel restart; public inventory contains only receipts.
use super::*;

type IssuedGenerations = BTreeMap<u64, Vec<String>>;

impl ProviderAccountProfileRegistry {
    pub(crate) fn prepare_account_copy(
        &self,
        owner: &str,
        materialization: &ProviderAccountMaterialization,
        kind: ProviderAccountMaterializationTargetKind,
        machine: &str,
        kernel: &str,
    ) -> Result<ProviderAccountCopyExpectation, DaemonError> {
        let expected = ProviderAccountCopyExpectation::from_materialization(materialization)?;
        self.remember_issued_account_copy(owner, &expected, kind, machine, kernel)?;
        Ok(expected)
    }

    pub(crate) fn remember_issued_account_copy(
        &self,
        owner: &str,
        expected: &ProviderAccountCopyExpectation,
        kind: ProviderAccountMaterializationTargetKind,
        machine: &str,
        kernel: &str,
    ) -> Result<(), DaemonError> {
        if expected.renewable_services.is_empty() {
            return Ok(()); // Metadata-only setup-token and API-key paths have no login-copy receipt.
        }
        self.get(owner, &expected.provider, &expected.source_account_id)?;
        if self.copy_identity.as_ref() != Some(&expected.source)
            || expected.generated_at_ms == 0
            || machine.trim().is_empty()
            || kernel.trim().is_empty()
        {
            return Err(registry_error(
                "issue account copy",
                "copy request lacks authoritative placement or generation",
            ));
        }
        let _guard = self.write_document()?;
        let path = self.issued_copy_path(owner, expected, kind, machine, kernel)?;
        let mut pending = read_issued_generations(&path)?.unwrap_or_default();
        if pending
            .get(&expected.generated_at_ms)
            .is_some_and(|services| services != &expected.renewable_services)
        {
            return Err(registry_error(
                "issue account copy",
                "copy generation already has different renewable services",
            ));
        }
        pending.insert(
            expected.generated_at_ms,
            expected.renewable_services.clone(),
        );
        let directory = path.parent().unwrap();
        fs::create_dir_all(directory).map_err(registry_io("issue account copy"))?;
        set_private_dir_permissions(directory)?;
        write_issued_generations(&path, &pending)
    }

    pub(super) fn issued_copy_matches(
        &self,
        owner: &str,
        expected: &ProviderAccountCopyExpectation,
        kind: ProviderAccountMaterializationTargetKind,
        machine: &str,
        kernel: &str,
        copy: &ProviderAccountCopyMetadata,
    ) -> Result<bool, DaemonError> {
        let _guard = self.read_document()?;
        let path = self.issued_copy_path(owner, expected, kind, machine, kernel)?;
        Ok(read_issued_generations(&path)?.is_some_and(|pending| {
            pending.get(&copy.copied_at_ms) == Some(&copy.renewable_services)
        }))
    }

    pub(super) fn finish_issued_account_copy(
        &self,
        owner: &str,
        expected: &ProviderAccountCopyExpectation,
        kind: ProviderAccountMaterializationTargetKind,
        machine: &str,
        kernel: &str,
        confirmed_generation: u64,
    ) -> Result<(), DaemonError> {
        let _guard = self.write_document()?;
        let path = self.issued_copy_path(owner, expected, kind, machine, kernel)?;
        let Some(mut pending) = read_issued_generations(&path)? else {
            return Ok(());
        };
        // A preserved-login response settles this attempt too. Keep newer concurrent
        // requests until their own acknowledgement so a lost response remains reconcilable.
        pending.retain(|generation, _| {
            *generation > confirmed_generation && *generation != expected.generated_at_ms
        });
        write_issued_generations(&path, &pending)
    }

    fn issued_copy_path(
        &self,
        owner: &str,
        expected: &ProviderAccountCopyExpectation,
        kind: ProviderAccountMaterializationTargetKind,
        machine: &str,
        kernel: &str,
    ) -> Result<PathBuf, DaemonError> {
        let scope = serde_json::to_vec(&(
            "issued-account-copy",
            owner,
            &expected.provider,
            &expected.source.machine_id,
            &expected.source.kernel_id,
            &expected.source_account_id,
            kind,
            machine,
            kernel,
        ))
        .map_err(|error| registry_error("issue account copy", error.to_string()))?;
        let directory = self.path.with_extension("copy-issued-generations");
        if path_entry_exists(&directory)? {
            validate_managed_directory(&directory, "issue account copy")?;
        }
        Ok(directory.join(format!("{:x}", Sha256::digest(scope))))
    }
}

fn read_issued_generations(path: &Path) -> Result<Option<IssuedGenerations>, DaemonError> {
    read_bounded_regular_file_no_follow(path, 1024 * 1024, "issued copy generations")?
        .map(|bytes| {
            serde_json::from_slice(&bytes).map_err(|_| {
                registry_error(
                    "issue account copy",
                    "invalid issued copy generation journal",
                )
            })
        })
        .transpose()
}

fn write_issued_generations(path: &Path, pending: &IssuedGenerations) -> Result<(), DaemonError> {
    let bytes = serde_json::to_vec(pending)
        .map_err(|error| registry_error("issue account copy", error.to_string()))?;
    atomic_write_private(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn review_ack_issued_generations_survive_restart_and_are_bound_to_receiving_placement() {
        let root = crate::test_support::TestWorktree::new("issued-copy-generations");
        let path = root.path().join("home/profiles.json");
        let registry = ProviderAccountProfileRegistry::open(&path)
            .unwrap()
            .with_machine_identity("home-machine", "home-kernel");
        let profile = registry.create_managed("owner", "codex", "Copy").unwrap();
        let mut materialization = ProviderAccountMaterialization {
            copy_source: registry.copy_identity.clone(),
            profile: ProviderAccountReplicaMetadata {
                owner_user_id: "owner".into(),
                provider: "codex".into(),
                profile_id: profile.profile_id.clone(),
                label: profile.label,
                origin: profile.origin,
                is_default: false,
            },
            files: vec![ProviderAccountMaterializationFile {
                relative_path: "auth.json".into(),
                contents_base64: base64::engine::general_purpose::STANDARD
                    .encode(br#"{"tokens":{"refresh_token":"synthetic"}}"#),
            }],
            generated_at_ms: 10,
        };
        let kind = ProviderAccountMaterializationTargetKind::Worker;
        registry
            .prepare_account_copy(
                "owner",
                &materialization,
                kind,
                "worker-machine",
                "worker-kernel",
            )
            .unwrap();
        drop(registry);
        let registry = ProviderAccountProfileRegistry::open(path)
            .unwrap()
            .with_machine_identity("home-machine", "home-kernel");
        materialization.generated_at_ms = 20;
        let expected = registry
            .prepare_account_copy(
                "owner",
                &materialization,
                kind,
                "worker-machine",
                "worker-kernel",
            )
            .unwrap();
        let receipt = ProviderAccountMaterializationStatus {
            target_kind: kind,
            target_ref: "worker-kernel".into(),
            state: ProviderAccountMaterializationState::Materialized,
            observed_at_ms: 1,
            last_error: None,
            copy: Some(ProviderAccountCopyMetadata {
                source_machine_id: "home-machine".into(),
                source_kernel_id: "home-kernel".into(),
                source_account_id: profile.profile_id.clone(),
                target_machine_id: "worker-machine".into(),
                target_kernel_id: "worker-kernel".into(),
                target_account_id: "receiving-default".into(),
                renewable_services: vec!["codex".into()],
                auth_state: ProviderAccountCopyAuthState::Authenticated,
                copied_at_ms: 10,
                warning_seen: false,
            }),
        };
        for attack in 0..4 {
            let mut receipt = receipt.clone();
            let copy = receipt.copy.as_mut().unwrap();
            let (kind, machine, kernel) = match attack {
                0 => {
                    copy.copied_at_ms = 11;
                    (kind, "worker-machine", "worker-kernel")
                }
                1 => {
                    copy.renewable_services = vec!["attacker".into()];
                    (kind, "worker-machine", "worker-kernel")
                }
                2 => {
                    copy.target_machine_id = "other-machine".into();
                    copy.target_kernel_id = "other-kernel".into();
                    receipt.target_ref = "other-kernel".into();
                    (kind, "other-machine", "other-kernel")
                }
                _ => {
                    receipt.target_kind = ProviderAccountMaterializationTargetKind::Slice;
                    (
                        ProviderAccountMaterializationTargetKind::Slice,
                        "worker-machine",
                        "worker-kernel",
                    )
                }
            };
            assert!(
                registry
                    .record_confirmed_account_copy(
                        "owner",
                        &expected,
                        kind,
                        machine,
                        kernel,
                        "receiving-default",
                        receipt
                    )
                    .is_err(),
                "unissued generation/placement/services attack {attack} was accepted"
            );
        }
        registry
            .record_confirmed_account_copy(
                "owner",
                &expected,
                kind,
                "worker-machine",
                "worker-kernel",
                "receiving-default",
                receipt.clone(),
            )
            .unwrap();
        assert_eq!(
            registry
                .get("owner", "codex", &profile.profile_id)
                .unwrap()
                .installed_account_id_at(kind, "worker-kernel"),
            Some("receiving-default")
        );
        assert!(!registry
            .issued_copy_matches(
                "owner",
                &expected,
                kind,
                "worker-machine",
                "worker-kernel",
                receipt.copy.as_ref().unwrap()
            )
            .unwrap());
        // A newer concurrent request cannot be retired by another attempt's preserved-login receipt.
        materialization.generated_at_ms = 30;
        registry
            .prepare_account_copy(
                "owner",
                &materialization,
                kind,
                "worker-machine",
                "worker-kernel",
            )
            .unwrap();
        registry
            .record_confirmed_account_copy(
                "owner",
                &expected,
                kind,
                "worker-machine",
                "worker-kernel",
                "receiving-default",
                receipt.clone(),
            )
            .unwrap();
        let mut newer = receipt.copy.clone().unwrap();
        newer.copied_at_ms = 30;
        assert!(registry
            .issued_copy_matches(
                "owner",
                &expected,
                kind,
                "worker-machine",
                "worker-kernel",
                &newer
            )
            .unwrap());
        let mut removed = receipt.clone();
        removed.copy.as_mut().unwrap().auth_state = ProviderAccountCopyAuthState::Removed;
        assert!(
            registry
                .record_confirmed_account_copy(
                    "owner",
                    &expected,
                    kind,
                    "worker-machine",
                    "worker-kernel",
                    "receiving-default",
                    removed.clone()
                )
                .is_err(),
            "a removal tombstone cannot acknowledge a successful installation"
        );
        registry
            .apply_remote_account_copy_observation(
                "owner",
                "codex",
                &profile.profile_id,
                kind,
                "worker-machine",
                "worker-kernel",
                &ProviderAccountCopyObservation {
                    provider: "codex".into(),
                    status: removed,
                },
            )
            .unwrap();
        assert!(!registry
            .get("owner", "codex", &profile.profile_id)
            .unwrap()
            .is_installed_at(kind, "worker-kernel"));
    }
}
