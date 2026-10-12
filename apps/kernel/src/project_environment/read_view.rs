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
        operation
            .per_item_results
            .retain(|result| result.requirement_id.starts_with("detect:"));
        operation.per_item_results.truncate(32);
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
}
