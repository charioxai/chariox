//! MP-08 / MP-10 / MP-11: private dispatch authority; never supplied by a model or client JSON.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SkillTurnAuthority {
    pub catalog_revision: u64,
    pub project_id: String,
    pub provider_run_id: String,
    pub context_generation: String,
    pub explicit_skill_ids: Vec<String>,
    /// Conservative token allowance shared by explicit bodies and later loads.
    pub body_budget_tokens: u64,
    /// Reservations are not delivery acknowledgements or public Loaded records.
    #[serde(default)]
    pub reserved_body_tokens: BTreeMap<String, u64>,
    #[serde(default)]
    pub pending_loads: Vec<super::v2::SkillLoadRecord>,
    #[serde(default)]
    pub prompt_delivery_skill_ids: std::collections::BTreeSet<String>,
    /// Leased turns: worker-verified package roots by skill ID, bound after the transfer receipt.
    #[serde(default)]
    pub remote_roots: BTreeMap<String, String>,
    /// Loads a provider's own skill tool settles by name (OpenCode's `skill` tool):
    /// the name the harness invokes, mapped to the pending load's skill ID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub native_tool_loads: BTreeMap<String, String>,
    /// A leased turn that reloads after a worker compaction: acknowledged with its first delivery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_revision: Option<u64>,
}
/// Leased turns have no home provider run; the run ID names the lease instead.
pub(crate) const LEASED_RUN_PREFIX: &str = "leased:";
impl SkillTurnAuthority {
    pub(crate) fn is_leased(&self) -> bool {
        self.provider_run_id.starts_with(LEASED_RUN_PREFIX)
    }
    pub(crate) fn reserve_body(&mut self, skill_id: &str, tokens: u64) -> Result<(), &'static str> {
        if let Some(previous) = self.reserved_body_tokens.get(skill_id) {
            return if *previous == tokens {
                Ok(())
            } else {
                Err("pinned skill body changed; start a new turn")
            };
        }
        let used = self.reserved_body_tokens.values().try_fold(tokens, |total, amount|total.checked_add(*amount))
            .ok_or("selected skill bodies exceed remaining turn budget; select fewer or smaller skills")?;
        if used > self.body_budget_tokens {
            return Err("selected skill bodies exceed remaining turn budget; select fewer or smaller skills");
        }
        self.reserved_body_tokens.insert(skill_id.into(), tokens);
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skill_turn_body_budget_accumulates_and_retries_do_not_reserve_twice() {
        let mut turn = SkillTurnAuthority {
            catalog_revision: 1,
            project_id: "project".into(),
            provider_run_id: "run".into(),
            context_generation: "turn".into(),
            explicit_skill_ids: vec![],
            body_budget_tokens: 10,
            reserved_body_tokens: BTreeMap::new(),
            pending_loads: vec![],
            prompt_delivery_skill_ids: Default::default(),
            remote_roots: Default::default(),
            native_tool_loads: Default::default(),
            compaction_revision: None,
        };
        turn.reserve_body("first", 5).unwrap();
        turn.reserve_body("first", 5).unwrap();
        assert!(turn.reserve_body("first", 6).is_err());
        assert!(turn.reserve_body("second", 6).is_err());
        assert_eq!(turn.reserved_body_tokens.len(), 1);
        turn.reserve_body("second", 5).unwrap();
        assert!(turn.reserve_body("overflow", u64::MAX).is_err());
        assert_eq!(turn.reserved_body_tokens.len(), 2);
    }
    #[test]
    fn persisted_authority_without_remote_roots_still_decodes() {
        let old = serde_json::json!({
            "catalog_revision": 2, "project_id": "p", "provider_run_id": "leased:agent",
            "context_generation": "s:p:remote", "explicit_skill_ids": [], "body_budget_tokens": 9
        });
        let turn: SkillTurnAuthority = serde_json::from_value(old).unwrap();
        assert!(turn.remote_roots.is_empty());
        assert!(turn.is_leased());
    }
}
