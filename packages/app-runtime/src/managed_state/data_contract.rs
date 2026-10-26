//! What an update may do to an installation's durable data
//! (docs/APPS_V2_CONTRACTS.md §7). Data belongs to the installation, so dev and
//! release data are separate and a channel switch or subscription change keeps
//! them. These pure rules decide acceptance and recovery; the snapshot and
//! journal mechanisms (`migration.rs`, the private volume) carry them out.
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "acceptance", rename_all = "snake_case")]
pub enum MigrationAcceptance {
    /// The release expects the data's version: nothing to run.
    AlreadyCurrent,
    /// Run each step in order, each as `from` to `to`, in the restricted
    /// migration world, with every old writer fenced.
    Forward { steps: Vec<(u32, u32)> },
    /// The data is newer than the release. Refused; installing it would need a
    /// reverse migration, never a silent drop of newer data.
    Downgrade {
        data_version: u32,
        release_target: u32,
    },
}

/// `data_version` is the schema version of the installation's data (zero when
/// clean); `release_target` is the release's `migrations.targetVersion`.
pub fn accept_migration(data_version: u32, release_target: u32) -> MigrationAcceptance {
    use std::cmp::Ordering::*;
    match data_version.cmp(&release_target) {
        Equal => MigrationAcceptance::AlreadyCurrent,
        Less => MigrationAcceptance::Forward {
            steps: (data_version..release_target)
                .map(|from| (from, from + 1))
                .collect(),
        },
        Greater => MigrationAcceptance::Downgrade {
            data_version,
            release_target,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStage {
    BeforeCommit,
    AfterCommit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Recovery {
    /// Structured state and private files return to the snapshot and the prior
    /// working release runs again.
    RestorePriorRelease,
    /// New writes exist: a compatible reverse (or forward) migration repairs.
    CompatibleMigration,
    /// No compatible migration: block with the data preserved and say so.
    BlockedPreservingData,
}

/// A failed migration, startup or health check. Accepted writes and effect
/// history after commit are never discarded to make a rollback look clean.
pub fn recovery(stage: UpdateStage, compatible_migration_available: bool) -> Recovery {
    match (stage, compatible_migration_available) {
        (UpdateStage::BeforeCommit, _) => Recovery::RestorePriorRelease,
        (UpdateStage::AfterCommit, true) => Recovery::CompatibleMigration,
        (UpdateStage::AfterCommit, false) => Recovery::BlockedPreservingData,
    }
}
