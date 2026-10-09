//! MP-02 / MP-03 / MP-08: Idempotent legacy read-through; no discovery, migration or execution.
use super::*;
use sha2::{Digest, Sha256};

pub(crate) fn metadata_digest(value: &impl serde::Serialize) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("metadata encodes"))
    )
}

pub fn project_environment_snapshot(
    project: &crate::session::RuntimeProject,
    lineage: EnvironmentLineage,
    folder_ids: &std::collections::BTreeMap<String, String>,
    legacy: Option<&StoredProjectEnvironment>,
) -> ProjectEnvironment {
    let mut folders: Vec<_> = project
        .workspace_ids()
        .iter()
        .map(|workspace| {
            let folder_id = folder_ids[workspace].clone();
            let label = std::path::Path::new(workspace)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(workspace)
                .to_string();
            EnvironmentFolder {
                portable_folder_key: folder_id.clone(),
                folder_id,
                label,
                local_workspace_binding: workspace.clone(),
                optional_git: None,
                requirements: vec![],
            }
        })
        .collect();
    let mut project_requirements = vec![];
    if let Some(definition) = project.environment_definition() {
        let digest = definition.digest();
        project_requirements.push(Requirement {
            requirement_id: format!("{}:setup", lineage.environment_id),
            title: "Saved project setup".into(),
            scope: RequirementScope::Project,
            origins: vec![RequirementOrigin::Migrated {
                source: "project_environment_definition".into(),
                digest: digest.clone(),
            }],
            spec: RequirementSpec::SetupChecks {
                commands: vec![],
                probes: vec![],
                boundary: EnvironmentExecutionBoundary::DisposableWorker,
                source_path: definition.source_path.clone(),
                source_digest: Some(digest.clone()),
                legacy: Some(LegacySetupReference {
                    definition_digest: digest,
                    origin: definition.origin,
                    source: definition.source,
                    target_platform: definition.target_platform.clone(),
                    source_path: definition.source_path.clone(),
                    inputs: definition.inputs.clone(),
                    path_entries: definition.path_entries.clone(),
                    setup_step_kinds: definition.setup_steps.iter().map(|s| s.kind).collect(),
                    setup_command_digests: definition
                        .setup_steps
                        .iter()
                        .map(|s| metadata_digest(&s.command))
                        .collect(),
                    validation_command_digests: definition
                        .validation_commands
                        .iter()
                        .map(metadata_digest)
                        .collect(),
                }),
            },
            depends_on: vec![],
            platform_variants: vec![],
            required: true,
            legacy_entry: None,
        });
    }
    let mut proposals = vec![];
    if let Some(stored) = legacy {
        for entry in &stored.manifest.entries {
            let folder = folders
                .iter_mut()
                .find(|folder| folder.local_workspace_binding == entry.workspace_id);
            let scope = folder
                .as_ref()
                .map(|f| RequirementScope::Folder {
                    folder_id: f.folder_id.clone(),
                })
                .unwrap_or(RequirementScope::Project);
            let spec = match entry.classification {
                ProjectEnvironmentClassification::NonSecret => RequirementSpec::Variables {
                    name: entry.name.clone(),
                    locator: entry.locator.clone(),
                },
                ProjectEnvironmentClassification::Secret => RequirementSpec::Secrets {
                    name: entry.name.clone(),
                    vault: match &entry.locator {
                        ProjectEnvironmentLocator::Vault { service, key } => {
                            Some(EnvironmentVaultReference {
                                service: service.clone(),
                                key: key.clone(),
                            })
                        }
                        _ => None,
                    },
                },
            };
            let mut origins = vec![RequirementOrigin::Migrated {
                source: "project_environment_manifest".into(),
                digest: stored.manifest.evidence_digest.clone(),
            }];
            if let Some(folder) = &folder {
                origins.extend(entry.uses.iter().map(|usage| RequirementOrigin::Detected {
                    folder_id: folder.folder_id.clone(),
                    relative_path: usage.path.clone(),
                    line: Some(usage.line),
                    evidence_digest: stored.manifest.evidence_digest.clone(),
                }));
            }
            let requirement = Requirement {
                requirement_id: project_environment_item_id(
                    &lineage.environment_id,
                    &format!("{}\0{}", entry.workspace_id, entry.name),
                ),
                title: entry.name.clone(),
                scope,
                origins,
                spec,
                depends_on: vec![],
                platform_variants: vec![],
                required: !entry.excluded,
                legacy_entry: Some(entry.clone()),
            };
            if let Some(folder) = folder {
                folder.requirements.push(requirement);
            } else {
                project_requirements.push(requirement);
            }
        }
        for file in &stored.manifest.private_files {
            let folder = folders
                .iter_mut()
                .find(|folder| folder.local_workspace_binding == file.workspace_id);
            let Some(folder) = folder else {
                continue;
            };
            folder.requirements.push(Requirement {
                requirement_id: project_environment_item_id(
                    &lineage.environment_id,
                    &format!("file\0{}\0{}", file.workspace_id, file.path),
                ),
                title: file.path.clone(),
                scope: RequirementScope::Folder {
                    folder_id: folder.folder_id.clone(),
                },
                origins: vec![RequirementOrigin::Migrated {
                    source: "private_file_decision".into(),
                    digest: stored.manifest.evidence_digest.clone(),
                }],
                spec: RequirementSpec::Files {
                    folder_id: folder.folder_id.clone(),
                    entries: vec![EnvironmentFile {
                        relative_path: file.path.clone(),
                        kind: EnvironmentFileKind::File,
                        content_digest: stored
                            .evidence
                            .files
                            .get(&file.workspace_id)
                            .and_then(|files| files.get(&file.path))
                            .cloned(),
                        folder_id: folder.folder_id.clone(),
                        user_selected: true,
                        git_ignored: None,
                        byte_count: stored
                            .evidence
                            .private_inventory
                            .get(&file.workspace_id)
                            .and_then(|files| files.get(&file.path))
                            .copied(),
                        credential_filter_verdict: if file.secret_looking {
                            EnvironmentCredentialFilterVerdict::NeedsVault
                        } else {
                            EnvironmentCredentialFilterVerdict::NotChecked
                        },
                        transfer_inclusion: if file.bring {
                            EnvironmentTransferInclusion::ReviewRequired
                        } else {
                            EnvironmentTransferInclusion::Exclude
                        },
                        reason: Some(file.reason.clone()),
                        secret_looking: file.secret_looking,
                    }],
                },
                depends_on: vec![],
                platform_variants: vec![],
                required: file.bring,
                legacy_entry: None,
            });
        }
        for (source, hints) in [
            ("toolchain_hints", &stored.manifest.toolchain_hints),
            ("package_hints", &stored.manifest.package_hints),
            ("service_hints", &stored.manifest.service_hints),
        ] {
            for hint in hints {
                let id = project_environment_item_id(
                    &lineage.environment_id,
                    &format!("{source}\0{hint}"),
                );
                // Hints remain proposals; they never silently become requirements.
                if proposals
                    .iter()
                    .any(|p: &EnvironmentProposal| p.proposal_id == id)
                {
                    continue;
                }
                let spec = if source == "service_hints" {
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
                proposals.push(EnvironmentProposal {
                    proposal_id: id.clone(),
                    requirement: Requirement {
                        requirement_id: id,
                        title: hint.clone(),
                        scope: RequirementScope::Project,
                        origins: vec![RequirementOrigin::DetectedMetadata {
                            source: source.into(),
                            reference: stored.manifest.evidence_digest.clone(),
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
    let mut snapshot = ProjectEnvironment {
        schema_version: 1,
        delivered_capabilities: DeliveredEnvironmentCapabilities {
            enabled_environment_operations: vec![EnvironmentCapability::Get],
            supported_schema: 1,
        },
        local_project_id: project.id().into(),
        lineage,
        revision: 0,
        content_digest: String::new(),
        parent_revision_digest: None,
        evidence_digest: legacy.map(|s| s.manifest.evidence_digest.clone()),
        project_requirements,
        folders,
        proposals,
        reviewed_at_ms: None,
        reviewed_by: None,
        observations: vec![],
        operations: vec![],
        legacy_manifest: legacy.map(|s| s.manifest.clone()),
        legacy_reviewed_manifest: legacy.and_then(|s| s.reviewed_manifest.clone()),
        legacy_review: legacy.and_then(|s| s.last_review.clone()),
    };
    refresh_environment_content_digest(&mut snapshot);
    snapshot
}

pub fn refresh_environment_content_digest(snapshot: &mut ProjectEnvironment) {
    // Machine observations and target-owned absolute paths are not portable content identity.
    snapshot.content_digest = metadata_digest(&(
        &snapshot.lineage,
        snapshot
            .project_requirements
            .iter()
            .map(requirement_specification)
            .collect::<Vec<_>>(),
        snapshot
            .folders
            .iter()
            .map(|f| {
                (
                    &f.folder_id,
                    f.requirements
                        .iter()
                        .map(requirement_specification)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>(),
        snapshot
            .proposals
            .iter()
            .map(|proposal| {
                (
                    &proposal.proposal_id,
                    requirement_specification(&proposal.requirement),
                )
            })
            .collect::<Vec<_>>(),
        &snapshot.evidence_digest,
    ));
}

// MP-08/MP-10: allowlist specification fields; keep legacy readiness in the response only.
fn requirement_specification(requirement: &Requirement) -> serde_json::Value {
    serde_json::json!({
        "requirement_id": requirement.requirement_id,
        "title": requirement.title,
        "scope": requirement.scope,
        "origins": requirement.origins,
        "spec": requirement.spec,
        "depends_on": requirement.depends_on,
        "platform_variants": requirement.platform_variants,
        "required": requirement.required,
        "legacy_entry": requirement.legacy_entry.as_ref().map(|entry| serde_json::json!({
            "name": entry.name,
            "workspace_id": entry.workspace_id,
            "kind": entry.kind,
            "classification": entry.classification,
            "excluded": entry.excluded,
            "uses": entry.uses,
            "locator": entry.locator,
        })),
    })
}
