//! MP-08 / MP-10 / MP-11: home-owned instruction catalogs; no capability grants.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SKILL_CATALOG_SCHEMA: u32 = 2;
pub const SKILLS_MIN_LOCAL_PROTOCOL: u32 = 533;
pub const SKILLS_MIN_PEER_PROTOCOL: u32 = 141;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillScope {
    Chariox,
    Project { project_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillState {
    Auto,
    Manual,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillFileManifest {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub executable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillProvenance {
    pub kind: String,
    pub source_machine: Option<String>,
    /// Sanitized locator: no URL credentials, query or fragment.
    pub locator: Option<String>,
    pub source_revision: Option<String>,
    pub provider: Option<String>,
    pub account_profile: Option<String>,
    pub installed_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillRecord {
    pub id: String,
    pub name: String,
    pub scope: SkillScope,
    pub package_id: Option<String>,
    pub description: String,
    pub short_description: Option<String>,
    pub default_state: SkillState,
    pub state_override: Option<SkillState>,
    pub digest: String,
    pub declared_version: Option<String>,
    pub files: Vec<SkillFileManifest>,
    pub provenance: SkillProvenance,
    /// Unsupported provider metadata is informational and never executes.
    pub compatibility_notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillPackageRecord {
    pub id: String,
    pub name: String,
    pub scope: SkillScope,
    pub custom: bool,
    pub source_locator: Option<String>,
    pub source_revision: Option<String>,
    /// Upstream identity -> stable skill ID; not membership authority.
    pub upstream_members: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillCatalog {
    pub schema: u32,
    /// Home-wide monotonic revision: serializes collisions across all Projects.
    pub revision: u64,
    pub scope: SkillScope,
    pub skills: Vec<SkillRecord>,
    pub packages: Vec<SkillPackageRecord>,
    pub chariox_overrides: BTreeMap<String, SkillState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillSelection {
    Chariox {
        skill_id: String,
        name: String,
        catalog_revision: u64,
    },
    Native {
        entry_id: String,
        name: String,
        provider: String,
        account_profile: String,
        execution_id: String,
        inventory_revision: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeSkillExposure {
    Confirmed,
    Unverified,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSkillEntry {
    pub selection: SkillSelection,
    pub description: String,
    pub digest: Option<String>,
    pub exposure: NativeSkillExposure,
    pub default_state: SkillState,
    pub complete_manifest: bool,
    pub supports_load_ack: bool,
    pub discovery_error: Option<String>,
}

/// Read-only details of the focused harness entry, never an installed revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSkillProvenance {
    pub source_machine: Option<String>,
    pub locator: Option<String>,
    pub provider_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSkillDetails {
    pub entry: NativeSkillEntry,
    pub provenance: NativeSkillProvenance,
    pub body: Option<String>,
    pub files: Vec<SkillFileManifest>,
    pub declared_version: Option<String>,
    pub compatibility_notes: Vec<String>,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillDelivery {
    Chariox,
    ProvidedBy {
        provider: String,
        entry_id: String,
    },
    Conflict {
        provider: String,
        entry_id: String,
        native_digest: String,
    },
    Unverified {
        provider: String,
        entry_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillProjection {
    pub skill: SkillRecord,
    pub effective_state: SkillState,
    pub project_override: Option<SkillState>,
    pub delivery: SkillDelivery,
    /// One picker row for exact duplicates; native entry is folded into this row.
    pub also_in_provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillLoadReason {
    Explicit,
    Model,
    ConfirmedNative,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillLoadRecord {
    pub skill_id: String,
    pub digest: String,
    pub session_id: String,
    pub agent_id: String,
    pub run_id: String,
    pub prompt_id: String,
    pub reason: SkillLoadReason,
    pub context_generation: String,
    /// Only acknowledged delivery records appear here; availability is not load.
    pub delivered_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillInstallSource {
    Folder {
        path: String,
    },
    Content {
        skill_md: String,
    },
    Url {
        url: String,
        pinned_revision: Option<String>,
    },
    Native {
        selection: SkillSelection,
    },
    /// One skill directory of a public git repository, pinned to the commit the
    /// owner reviewed through `ResolveRepository`. Owner-initiated only.
    Repository {
        url: String,
        revision: String,
        /// Directory relative to the repository root; empty for a root skill.
        path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillCollisionAction {
    Reject,
    Rename { name: String },
    Replace { skill_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillCatalogAction {
    Install {
        source: SkillInstallSource,
        collision: SkillCollisionAction,
        package_id: Option<String>,
    },
    Update {
        skill_id: String,
        source: SkillInstallSource,
        expected_digest: String,
    },
    Remove {
        skill_id: String,
    },
    SetState {
        skill_id: String,
        state: Option<SkillState>,
    },
    CreatePackage {
        package_id: String,
        name: String,
    },
    RenamePackage {
        package_id: String,
        name: String,
    },
    Regroup {
        skill_id: String,
        package_id: Option<String>,
    },
    RemovePackage {
        package_id: String,
        remove_skills: bool,
    },
    SetPackageState {
        package_id: String,
        state: Option<SkillState>,
    },
    Move {
        skill_id: String,
        destination: SkillScope,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillCatalogMutation {
    pub scope: SkillScope,
    pub expected_revision: u64,
    /// Reviewed batch is validated in full and published with ONE revision.
    pub actions: Vec<SkillCatalogAction>,
}

/// Home-owned preview identity. Mutation accepts its ID only; the home service
/// verifies owner/scope/revision and the canonical mutation hash, and publishes
/// the staged reviewed bytes without fetching a newer source at approval time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillReviewReceipt {
    pub id: String,
    pub expires_at_ms: u64,
    pub mutation_sha256: String,
    pub sources: Vec<SkillReviewedSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillReviewedSource {
    pub action_index: u32,
    pub skill_id: String,
    pub digest: String,
    pub source_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillsV2Request {
    Catalog {
        project_id: Option<String>,
        agent_id: Option<String>,
    },
    Get {
        skill_id: String,
        project_id: Option<String>,
    },
    NativeInventory {
        agent_id: String,
        refresh: bool,
    },
    NativeDetails {
        agent_id: String,
        selection: SkillSelection,
    },
    Loads {
        session_id: String,
        agent_id: Option<String>,
    },
    Mutate {
        mutation: SkillCatalogMutation,
        review_receipt_id: String,
    },
    Preview {
        mutation: SkillCatalogMutation,
    },
    /// Fetch a public git repository and list the skills it carries at its
    /// current commit. Owner-only; nothing is installed.
    ResolveRepository {
        url: String,
    },
    ResolveRepositoryUpdate {
        package_id: String,
    },
    List {
        session_id: String,
        agent_id: String,
        cursor: Option<String>,
        limit: u32,
    },
    Load {
        session_id: String,
        agent_id: String,
        skill_id: String,
    },
    Submit {
        session_id: String,
        attachment_id: String,
        target_agent_id: String,
        prompt: String,
        attachments: Vec<crate::session::PromptAttachment>,
        selections: Vec<SkillSelection>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillsV2Response {
    Catalog {
        revision: u64,
        skills: Vec<SkillProjection>,
        packages: Vec<SkillPackageRecord>,
        native: Vec<NativeSkillEntry>,
        metadata_budget_percent: f64,
    },
    Detail {
        skill: SkillProjection,
        body: String,
    },
    NativeInventory {
        entries: Vec<NativeSkillEntry>,
        inventory_revision: String,
    },
    NativeDetails {
        details: NativeSkillDetails,
    },
    Loads {
        records: Vec<SkillLoadRecord>,
    },
    Preview {
        expected_revision: u64,
        affected: Vec<SkillRecord>,
        collisions: Vec<String>,
        review_receipt: SkillReviewReceipt,
    },
    Repository {
        locator: String,
        /// Commit the listing was read at; installs pin exactly this commit.
        revision: String,
        skills: Vec<RepositorySkill>,
        /// SKILL.md directories that cannot be installed, with the reason.
        skipped: Vec<RepositorySkillSkip>,
    },
    RepositoryUpdate {
        package_id: String,
        locator: String,
        from_revision: String,
        revision: String,
        changed: Vec<RepositorySkillUpdate>,
        added: Vec<RepositorySkill>,
        removed: Vec<RepositorySkillMember>,
        unchanged: Vec<RepositorySkillMember>,
        skipped: Vec<RepositorySkillSkip>,
        mutation: SkillCatalogMutation,
    },
    Published {
        revision: u64,
        changed_skill_ids: Vec<String>,
        removed_skill_ids: Vec<String>,
    },
    Page {
        revision: u64,
        skills: Vec<SkillProjection>,
        next_cursor: Option<String>,
    },
    Loaded {
        skill_id: String,
        digest: String,
        body: String,
        root: String,
        files: Vec<SkillFileManifest>,
    },
    Submitted {
        outcome: crate::session::PromptSubmissionOutcome,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositorySkill {
    pub path: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositorySkillSkip {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositorySkillMember {
    pub path: String,
    pub skill_id: String,
    pub name: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositorySkillUpdate {
    pub member: RepositorySkillMember,
    pub digest: String,
}

/// MP-08 / MP-10 / MP-11: shared event payload for catalog and load invalidation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillCatalogEvent {
    Changed {
        revision: u64,
        scope: SkillScope,
        changed_skill_ids: Vec<String>,
        removed_skill_ids: Vec<String>,
    },
    Loaded {
        record: SkillLoadRecord,
    },
}

/// Authenticated lease context remains on EnsureRemoteSkillPackages.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillTransferManifest {
    pub catalog_revision: u64,
    pub project_id: Option<String>,
    pub skills: Vec<SkillTransferEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillTransferEntry {
    pub skill_id: String,
    pub digest: String,
    pub scope: SkillScope,
    pub state: SkillState,
}

/// Project snapshot with only the exported package members and Chariox dependencies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillEnvironmentSnapshot {
    pub schema: u32,
    pub source_revision: u64,
    pub project: SkillCatalog,
    pub chariox_dependencies: Vec<SkillRecord>,
    pub dependency_packages: Vec<SkillPackageRecord>,
}

/// Provider adapters consume a pinned turn, never independently resolve a catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillTurnContext {
    pub catalog_revision: u64,
    pub context_generation: String,
    pub metadata: String,
    pub metadata_tokens_upper_bound: u64,
    pub metadata_budget_tokens: u64,
    pub omitted_auto_count: u32,
    pub loaded: Vec<SkillResolvedBody>,
    pub native_selections: Vec<SkillSelection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillResolvedBody {
    pub skill_id: String,
    pub name: String,
    pub digest: String,
    pub root: String,
    pub body: String,
    pub files: Vec<SkillFileManifest>,
    pub reason: SkillLoadReason,
    pub delivery: SkillDelivery,
}
