//! MP-08 / MP-10 / MP-11: Validated metadata-only utility output stays reviewable.
use super::*;

impl EnvironmentDetection {
    // Deterministic proposals precede appended provider hints. Keep that priority
    // while reserving the remaining cache budget for operation results.
    fn bound_model_proposals(&mut self) {
        let mut bytes = 2usize;
        let mut omitted = 0usize;
        self.proposals.retain(|proposal| {
            let extra = super::detect_importers::encoded_size(proposal).saturating_add(1);
            if bytes.saturating_add(extra) > super::detect_importers::MAX_PROPOSAL_BYTES {
                omitted += 1;
                false
            } else {
                bytes += extra;
                true
            }
        });
        if omitted > 0 {
            self.skips.push(EnvironmentItemResult {
                requirement_id: "detect:model-proposal-bytes".into(),
                status: EnvironmentObservationStatus::NeedsYourInput,
                reason_code: "proposal_byte_limit".into(),
                safe_summary: format!(
                    "{omitted} provider proposals omitted · retained proposals limited to 2 MiB"
                ),
                receipt_ids: vec![],
            });
        }
    }
    pub fn reuse_model_metadata(
        &mut self,
        cache: &EnvironmentDetectionCache,
        folders: &std::collections::BTreeSet<String>,
    ) {
        for previous in &cache.proposals {
            let RequirementScope::Folder { folder_id } = &previous.requirement.scope else {
                continue;
            };
            if !folders.contains(folder_id) {
                continue;
            }
            let metadata: Vec<_> = previous
                .requirement
                .origins
                .iter()
                .filter(|o| matches!(o, RequirementOrigin::DetectedMetadata { .. }))
                .cloned()
                .collect();
            if metadata.is_empty() {
                continue;
            }
            if let Some(current) = self
                .proposals
                .iter_mut()
                .find(|p| p.proposal_id == previous.proposal_id)
            {
                current.requirement.spec = previous.requirement.spec.clone();
                current.requirement.origins.extend(metadata);
            } else if self.proposals.len() < 2048 {
                self.proposals.push(previous.clone());
            }
        }
        self.bound_model_proposals();
        self.proposals
            .sort_by(|a, b| a.proposal_id.cmp(&b.proposal_id));
    }
    pub fn merge_manifest(
        &mut self,
        environment_id: &str,
        input: &ProjectEnvironmentDiscoveryInput,
        manifest: &ProjectEnvironmentManifest,
    ) {
        for entry in &manifest.entries {
            for proposal in &mut self.proposals {
                if !matches!(&proposal.requirement.scope, RequirementScope::Folder { folder_id } if folder_id == &entry.workspace_id)
                {
                    continue;
                }
                let name = match &proposal.requirement.spec {
                    RequirementSpec::Secrets { name, .. }
                    | RequirementSpec::Variables { name, .. } => name,
                    _ => continue,
                };
                if name != &entry.name {
                    continue;
                }
                proposal.requirement.spec = match entry.classification {
                    ProjectEnvironmentClassification::Secret => RequirementSpec::Secrets {
                        name: entry.name.clone(),
                        vault: None,
                    },
                    ProjectEnvironmentClassification::NonSecret => RequirementSpec::Variables {
                        name: entry.name.clone(),
                        locator: ProjectEnvironmentLocator::Missing,
                    },
                };
                proposal
                    .requirement
                    .origins
                    .push(RequirementOrigin::DetectedMetadata {
                        source: "provider_classification".into(),
                        reference: manifest.evidence_digest.clone(),
                    });
            }
        }
        // Free-form hints cannot author a requirement, command, locator or capability.
        for (source, hints) in [
            ("provider_toolchain_hint", &manifest.toolchain_hints),
            ("provider_package_hint", &manifest.package_hints),
            ("provider_service_hint", &manifest.service_hints),
        ] {
            if hints.len() > 32 {
                self.skips.push(EnvironmentItemResult {
                    requirement_id: format!("detect:{source}"),
                    status: EnvironmentObservationStatus::NeedsYourInput,
                    reason_code: "utility_hints_bounded".into(),
                    safe_summary: "Provider hints limited to 32 per category; all require review"
                        .into(),
                    receipt_ids: vec![],
                });
            }
            for hint in hints.iter().take(32) {
                if !super::detect_index::safe_metadata(hint) {
                    continue;
                }
                for folder in input.changed_paths.keys() {
                    let id = project_environment_item_id(
                        environment_id,
                        &format!("{folder}\0{source}\0{hint}"),
                    );
                    if self.proposals.iter().any(|p| p.proposal_id == id) {
                        continue;
                    }
                    if self.proposals.len() >= 2048 {
                        let excluded =
                            super::detect_index::skip(folder, "[provider hint]", "proposal_limit");
                        if !self.skips.iter().any(|item| {
                            item.requirement_id == excluded.requirement_id
                                && item.reason_code == excluded.reason_code
                        }) {
                            self.skips.push(excluded);
                        }
                        continue;
                    }
                    let spec = if source == "provider_service_hint" {
                        RequirementSpec::Services {
                            identity: hint.clone(),
                            target_binding: None,
                            probe: None,
                            health_expectation: None,
                            depends_on: vec![],
                        }
                    } else {
                        RequirementSpec::Software {
                            identity: hint.clone(),
                            version_constraint: None,
                            platform: None,
                            install_scope: EnvironmentInstallScope::Project,
                            install_source: None,
                            detect_only: true,
                        }
                    };
                    self.proposals.push(EnvironmentProposal {
                        proposal_id: id.clone(),
                        requirement: Requirement {
                            requirement_id: id,
                            title: hint.clone(),
                            scope: RequirementScope::Folder {
                                folder_id: folder.clone(),
                            },
                            origins: vec![RequirementOrigin::DetectedMetadata {
                                source: source.into(),
                                reference: manifest.evidence_digest.clone(),
                            }],
                            spec,
                            depends_on: vec![],
                            platform_variants: vec![],
                            required: false,
                            legacy_entry: None,
                        },
                    });
                }
            }
        }
        self.bound_model_proposals();
        self.proposals
            .sort_by(|a, b| a.proposal_id.cmp(&b.proposal_id));
    }
}
