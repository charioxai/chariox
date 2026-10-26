//! One activation path and one approval rule (docs/APPS_V2_CONTRACTS.md §4).
use std::collections::BTreeSet;

use chariox_app_package::{
    v2::SignerPrincipal, ActionDeclaration, AssetAccess, Capabilities, ClipboardAccess,
    ExternalFileAccess,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::channel::{AcquisitionSource, InstallationChannel};

/// Every source converges on these two kernel operations after verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "entry", rename_all = "snake_case")]
pub enum ActivationEntry {
    /// `BeginAppInstall`: the first installation of this channel.
    Install,
    /// `BeginAppUpdate`, fenced on the generation the caller read.
    Update { expected_generation: u64 },
}

pub fn activation_entry(existing_generation: Option<u64>) -> ActivationEntry {
    match existing_generation {
        None => ActivationEntry::Install,
        Some(expected_generation) => ActivationEntry::Update {
            expected_generation,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum SourceRefusal {
    #[error("a dev build installs only as the dev channel, signed by the dev identity")]
    DevSourceNeedsDevChannel,
    #[error("the dev identity is not a publisher and cannot install a release")]
    DevSignerOnRelease,
    #[error("a publisher-signed package cannot replace a dev build")]
    PublisherSignerOnDev,
}

/// The dev channel takes the kernel's dev identity through `DevPush` only; the
/// release channel takes a publisher's signature from every other source.
pub fn check_source(
    source: AcquisitionSource,
    channel: InstallationChannel,
    signer: &SignerPrincipal,
) -> Result<(), SourceRefusal> {
    let dev_source = source == AcquisitionSource::DevPush;
    match (dev_source, channel, signer) {
        (true, InstallationChannel::Dev, SignerPrincipal::Dev)
        | (false, InstallationChannel::Release, SignerPrincipal::Publisher { .. }) => Ok(()),
        (true, InstallationChannel::Release, _) | (false, InstallationChannel::Dev, _) => {
            Err(SourceRefusal::DevSourceNeedsDevChannel)
        }
        (_, InstallationChannel::Release, SignerPrincipal::Dev) => {
            Err(SourceRefusal::DevSignerOnRelease)
        }
        (_, InstallationChannel::Dev, SignerPrincipal::Publisher { .. }) => {
            Err(SourceRefusal::PublisherSignerOnDev)
        }
    }
}

/// The normalized authority an owner approves: capabilities plus the protected
/// effect routes. Action names, descriptions and schemas are not authority, so
/// changing them, or adding an action inside existing authority, never asks
/// again. Identity enrollment and per-operation approval are separate checks.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AuthoritySet(BTreeSet<String>);

impl AuthoritySet {
    pub fn of(capabilities: &Capabilities, actions: &[ActionDeclaration]) -> Self {
        fn word<T: Serialize>(value: &T) -> String {
            serde_json::to_value(value)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_default()
        }
        let mut atoms = BTreeSet::new();
        for destination in &capabilities.network {
            for method in &destination.methods {
                atoms.insert(format!("network:{} {}", destination.origin, word(method)));
            }
        }
        let clipboard = |access: &ClipboardAccess| format!("clipboard:{}", word(access));
        let files = |access: &ExternalFileAccess| format!("files:{}", word(access));
        let asset = |kind: &str, access: &AssetAccess| format!("{kind}:{}", word(access));
        atoms.extend(capabilities.clipboard.iter().map(clipboard));
        atoms.extend(capabilities.external_files.iter().map(files));
        atoms.extend(capabilities.workflows.iter().map(|a| asset("workflows", a)));
        atoms.extend(capabilities.agents.iter().map(|a| asset("agents", a)));
        for connection in &capabilities.connections {
            for action in &connection.actions {
                atoms.insert(format!("connection:{}:{action}", connection.generator));
            }
        }
        for action in actions {
            for route in &action.effect_routes {
                let key = format!(
                    "{} {} {} {}",
                    route.origin,
                    word(&route.method),
                    route.path,
                    route.connection
                );
                if let Some(critical) = &action.critical_validation {
                    let kind = if critical.user_verification {
                        "verified"
                    } else {
                        "critical"
                    };
                    atoms.insert(format!("{kind}-route:{key}"));
                }
                atoms.insert(format!("route:{key}"));
            }
        }
        Self(atoms)
    }

    pub fn atoms(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    pub fn digest(&self) -> String {
        let canonical = serde_json::to_vec(&self.0).unwrap_or_default();
        format!("sha256:{:x}", Sha256::digest(canonical))
    }

    fn added_over(&self, approved: &Self) -> Vec<String> {
        self.0.difference(&approved.0).cloned().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum ApprovalRequirement {
    /// The durable decision covers the candidate: no prompt.
    Reuse,
    /// One shared interaction, listing exactly the new authority.
    Prompt { added: Vec<String> },
}

/// `approved` is the authority of the decision the installation already holds;
/// `None` is a first installation, which always asks.
pub fn approval_requirement(
    approved: Option<&AuthoritySet>,
    candidate: &AuthoritySet,
) -> ApprovalRequirement {
    let added = candidate.added_over(approved.unwrap_or(&AuthoritySet::default()));
    if approved.is_some() && added.is_empty() {
        ApprovalRequirement::Reuse
    } else {
        ApprovalRequirement::Prompt { added }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "refusal", rename_all = "snake_case")]
pub enum CommitRefusal {
    #[error("the approved authority changed since the decision; ask again")]
    AuthorityChanged { added: Vec<String> },
    #[error("publisher trust changed since the decision; verify again")]
    TrustChanged,
}

/// Evaluated again on the kernel writer at commit, against the authority and
/// publisher-trust revision as they are now, so a concurrent change cannot reuse
/// a stale approval.
pub fn commit_gate(
    approved_now: &AuthoritySet,
    candidate: &AuthoritySet,
    decided_trust_revision: u64,
    trust_revision_now: u64,
) -> Result<(), CommitRefusal> {
    if decided_trust_revision != trust_revision_now {
        return Err(CommitRefusal::TrustChanged);
    }
    let added = candidate.added_over(approved_now);
    if added.is_empty() {
        Ok(())
    } else {
        Err(CommitRefusal::AuthorityChanged { added })
    }
}
