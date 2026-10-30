//! MP-08 / MP-10 / MP-11: bounded reads; the durable aggregate remains authoritative.
use super::*;
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentViewRead {
    pub known_digest: Option<String>,
    pub proposals: Option<EnvironmentProposalPageRead>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentProposalPageRead {
    pub snapshot_digest: String,
    pub offset: usize,
    pub limit: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentReadView {
    /// Kernel-clock reference for freshness on clients running on another machine.
    pub observed_at_ms: u64,
    /// Covers Project identity, attachments, saved revision, discovery and current checks.
    pub snapshot_digest: String,
    /// None means an unchanged summary, or an explicit proposal-only page.
    pub environment: Option<ProjectEnvironment>,
    pub proposal_count: usize,
    pub proposal_offset: usize,
    pub next_proposal_offset: Option<usize>,
    pub proposals: Vec<EnvironmentProposal>,
    /// Full operation details remain available through the ordinary full read/operation lookup.
    pub operation_item_counts: Vec<usize>,
}

pub fn environment_read_view(
    mut environment: ProjectEnvironment,
    read: EnvironmentViewRead,
) -> Result<EnvironmentReadView, DaemonError> {
    let snapshot_digest = metadata_digest(&environment);
    let proposal_count = environment.proposals.len();
    let operation_item_counts = environment
        .operations
        .iter()
        .map(|o| o.per_item_results.len())
        .collect();
    let (offset, limit, page_only) = match &read.proposals {
        Some(page) => {
            if page.snapshot_digest != snapshot_digest {
                return Err(environment_error(
                    "Environment snapshot changed; refresh before reviewing proposals",
                ));
            }
            if page.limit == 0 || page.limit > 20 || page.offset > proposal_count {
                return Err(environment_error(
                    "Environment proposal page exceeds bounds",
                ));
            }
            (page.offset, page.limit, true)
        }
        None => (0, 0, false),
    };
    // A count bound alone is insufficient: a single requirement can contain many Files.
    // Keep every proposal reachable without truncating its provenance or Secret defaults.
    let mut proposals = vec![];
    let mut bytes = 0;
    for proposal in environment.proposals.iter().skip(offset).take(limit) {
        let size = serde_json::to_vec(proposal)
            .map_err(|_| environment_error("proposal encoding failed"))?
            .len();
        if !proposals.is_empty() && bytes + size > 32 * 1024 {
            break;
        }
        bytes += size;
        proposals.push(proposal.clone());
    }
    let next = offset + proposals.len();
    environment.proposals.clear();
    for operation in &mut environment.operations {
        operation.selected_items.clear();
        operation.per_item_opt_ins.clear();
        // MP-08 / MP-10 / MP-11: Detect exclusions use ordinary item IDs.
        // Keep bounded notes and concrete exclusions, with an explicit omitted
        // count that the shared Web/TUI skip renderer already understands.
        if operation.kind == EnvironmentOperationKind::Detect {
            let results = std::mem::take(&mut operation.per_item_results);
            let mut notes: Vec<_> = results
                .iter()
                .filter(|r| r.requirement_id.starts_with("detect:"))
                .collect();
            // Keep the utility outcome even if folder notes fill the whole page.
            notes.sort_by_key(|r| r.requirement_id != "detect:model");
            let skips: Vec<_> = results
                .iter()
                .filter(|r| !r.requirement_id.starts_with("detect:"))
                .collect();
            let note_count = notes.len().min(31 - skips.len().min(16));
            let skip_count = skips.len().min(31 - note_count);
            operation
                .per_item_results
                .extend(notes[..note_count].iter().map(|r| (*r).clone()));
            operation
                .per_item_results
                .extend(skips[..skip_count].iter().map(|r| (*r).clone()));
            let omitted_notes = notes.len() - note_count;
            let omitted_skips = skips.len() - skip_count;
            if omitted_notes + omitted_skips > 0 {
                operation.per_item_results.push(EnvironmentItemResult {
                    requirement_id: "detect:compact-omissions".into(),
                    status: EnvironmentObservationStatus::NeedsYourInput,
                    reason_code: "skip_summary_limit".into(),
                    safe_summary: format!("{omitted_skips} additional exclusions and {omitted_notes} detection notes omitted · narrow the selected folders for details"),
                    receipt_ids: vec![],
                });
            }
        } else {
            operation.per_item_results.clear();
        }
        operation.receipts.clear();
    }
    Ok(EnvironmentReadView {
        observed_at_ms: crate::session::unix_epoch_ms(),
        environment: if page_only || read.known_digest.as_deref() == Some(&snapshot_digest) {
            None
        } else {
            Some(environment)
        },
        snapshot_digest,
        proposal_count,
        proposal_offset: offset,
        next_proposal_offset: (next < proposal_count).then_some(next),
        proposals,
        operation_item_counts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn populated() -> ProjectEnvironment {
        let project = crate::session::RuntimeProject::new(
            "p",
            "local",
            "/plain",
            "Plain",
            crate::session::RuntimeProjectKind::Named,
        );
        let mut environment = project_environment_snapshot(
            &project,
            EnvironmentLineage {
                project_id: "lineage".into(),
                environment_id: "env".into(),
            },
            &std::collections::BTreeMap::from([("/plain".into(), "f".into())]),
            None,
        );
        environment.proposals = (0..1000)
            .map(|i| EnvironmentProposal {
                proposal_id: format!("proposal-{i}"),
                requirement: Requirement {
                    requirement_id: format!("req-{i}"),
                    title: "Secret reference".into(),
                    scope: RequirementScope::Folder {
                        folder_id: "f".into(),
                    },
                    origins: vec![],
                    spec: RequirementSpec::Secrets {
                        name: format!("KEY_{i}"),
                        vault: None,
                    },
                    depends_on: vec![],
                    platform_variants: vec![],
                    required: false,
                    legacy_entry: None,
                },
            })
            .collect();
        environment
    }
    #[test]
    fn compact_summary_and_pages_keep_every_secret_reachable() {
        let full = populated();
        let first = environment_read_view(
            full.clone(),
            EnvironmentViewRead {
                known_digest: None,
                proposals: None,
            },
        )
        .unwrap();
        assert_eq!(first.proposal_count, 1000);
        assert!(first.proposals.is_empty(), "opening is summary-only");
        assert_eq!(first.next_proposal_offset, Some(0));
        assert!(first.environment.as_ref().unwrap().proposals.is_empty());
        assert!(serde_json::to_vec(&first).unwrap().len() < 16 * 1024);
        let mut seen = first.proposals.clone();
        let mut next = first.next_proposal_offset;
        while let Some(offset) = next {
            let page = environment_read_view(
                full.clone(),
                EnvironmentViewRead {
                    known_digest: None,
                    proposals: Some(EnvironmentProposalPageRead {
                        snapshot_digest: first.snapshot_digest.clone(),
                        offset,
                        limit: 20,
                    }),
                },
            )
            .unwrap();
            assert!(page.environment.is_none());
            seen.extend(page.proposals);
            next = page.next_proposal_offset;
        }
        assert_eq!(seen, full.proposals);
        let same = environment_read_view(
            full.clone(),
            EnvironmentViewRead {
                known_digest: Some(first.snapshot_digest.clone()),
                proposals: None,
            },
        )
        .unwrap();
        assert!(same.environment.is_none());
        let mut cross_project = full.clone();
        cross_project.local_project_id.push('x');
        let mut revision = full.clone();
        revision.revision += 1;
        let mut attachment = full.clone();
        attachment.folders[0].local_workspace_binding.push('x');
        for changed in [cross_project, revision, attachment] {
            assert!(environment_read_view(
                changed,
                EnvironmentViewRead {
                    known_digest: None,
                    proposals: Some(EnvironmentProposalPageRead {
                        snapshot_digest: first.snapshot_digest.clone(),
                        offset: 8,
                        limit: 20
                    })
                }
            )
            .is_err());
        }
        let mut changed = full.clone();
        changed.proposals.pop();
        assert!(environment_read_view(
            changed,
            EnvironmentViewRead {
                known_digest: None,
                proposals: Some(EnvironmentProposalPageRead {
                    snapshot_digest: first.snapshot_digest,
                    offset: 8,
                    limit: 20
                })
            }
        )
        .is_err());
    }
    #[test]
    fn review0034_compact_detect_keeps_exclusions_and_discloses_omitted_count() {
        let mut env = populated();
        let operation:EnvironmentOperation=serde_json::from_value(serde_json::json!({
            "operation_id":"detect-review","attempt":1,"local_project_id":env.local_project_id,
            "revision_digest":env.content_digest,"target":{"machine_id":"machine","target_instance_generation":"boot","slice_ref":null},
            "kind":"detect","phase":"ready","selected_items":[],"per_item_opt_ins":[],"per_item_results":[],"created_at_ms":1,"updated_at_ms":2,"cancellation":null,"receipts":[],"recovery_state":{"kind":"settled"}
        })).unwrap();
        env.operations.push(operation);
        for index in 0..40 {
            env.operations[0]
                .per_item_results
                .push(super::super::detect_index::skip(
                    "folder",
                    &format!("evidence-{index}.txt"),
                    if index % 2 == 0 {
                        "protected_file"
                    } else {
                        "unsupported_encoding"
                    },
                ));
        }
        env.operations[0]
            .per_item_results
            .push(EnvironmentItemResult {
                requirement_id: "detect:folder".into(),
                status: EnvironmentObservationStatus::Ready,
                reason_code: "model_not_needed".into(),
                safe_summary: "Deterministic detection complete".into(),
                receipt_ids: vec![],
            });
        let compact = environment_read_view(
            env,
            EnvironmentViewRead {
                known_digest: None,
                proposals: None,
            },
        )
        .unwrap();
        assert_eq!(compact.operation_item_counts, [41]);
        let results = &compact.environment.unwrap().operations[0].per_item_results;
        assert!(results.iter().any(|r| r.reason_code == "protected_file"));
        assert!(results
            .iter()
            .any(|r| r.reason_code == "unsupported_encoding"));
        assert!(results.iter().any(|r| r.reason_code == "model_not_needed"));
        assert!(results.len() <= 32);
        assert!(results.iter().any(|r| r.reason_code == "skip_summary_limit"
            && r.safe_summary.contains("10 additional exclusions")));
    }
    // MP-08 / MP-10 / MP-11: unused exclusion slots belong to Detect disclosure.
    #[test]
    fn review0440_compact_detect_keeps_model_outcome_with_many_folder_notes() {
        let mut env = populated();
        let mut operation:EnvironmentOperation=serde_json::from_value(serde_json::json!({
            "operation_id":"detect-review","attempt":1,"local_project_id":env.local_project_id,"revision_digest":env.content_digest,
            "target":{"machine_id":"machine","target_instance_generation":"boot","slice_ref":null},"kind":"detect","phase":"ready",
            "selected_items":[],"per_item_opt_ins":[],"per_item_results":[],"created_at_ms":1,"updated_at_ms":2,"cancellation":null,"receipts":[],"recovery_state":{"kind":"settled"}
        })).unwrap();
        for index in 0..40 {
            operation.per_item_results.push(EnvironmentItemResult {
                requirement_id: format!("detect:folder:{index}"),
                status: EnvironmentObservationStatus::Ready,
                reason_code: "folder_note".into(),
                safe_summary: "Folder checked".into(),
                receipt_ids: vec![],
            });
        }
        operation.per_item_results.push(EnvironmentItemResult {
            requirement_id: "detect:model".into(),
            status: EnvironmentObservationStatus::Ready,
            reason_code: "model_completed".into(),
            safe_summary: "Utility completed".into(),
            receipt_ids: vec![],
        });
        env.operations = vec![operation];
        let compact = environment_read_view(
            env,
            EnvironmentViewRead {
                known_digest: None,
                proposals: None,
            },
        )
        .unwrap();
        let results = &compact.environment.unwrap().operations[0].per_item_results;
        assert!(results.len() <= 32);
        assert!(results.iter().any(|r| r.requirement_id == "detect:model"));
        assert!(
            results
                .iter()
                .filter(|r| r.reason_code == "folder_note")
                .count()
                >= 30
        );
        assert!(results
            .iter()
            .any(|r| r.reason_code == "skip_summary_limit"));
    }
}
