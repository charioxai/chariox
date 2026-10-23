//! MP-08 / MP-10 / MP-11: a home-pinned composer turn, materialized only at its worker.
use super::{leased_names::LeasedSkillNames, v2::*};
use serde::{Deserialize, Serialize};

pub(crate) const PREPARE_LEASED_SKILL_TURN: &str = "_chariox_prepare_native_skill_turn";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ComposerSkillPrompt {
    pub prompt_id: String,
    pub prompt: String,
    pub native: Vec<NativeSkillEntry>,
    pub cached_skills: std::collections::BTreeSet<CachedSkillDigest>,
    pub context_intact: bool,
    pub context_window: Option<u64>,
    pub context_tokens: Option<u64>,
}

/// One skill ID whose exact digest the worker holds in a verified execution cache.
/// Renamed copies share a digest, so a digest alone cannot say which IDs are present.
#[derive(Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub(crate) struct CachedSkillDigest {
    pub skill_id: String,
    pub digest: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ComposerSkillContext {
    pub manifest: SkillTransferManifest,
    pub packages: Vec<super::CharioxSkillPackage>,
    pub names: Option<LeasedSkillNames>,
    pub context: SkillTurnContext,
    pub auto_rows: Vec<SkillProjection>,
    pub budget_percent: f64,
    pub context_window: Option<u64>,
}
