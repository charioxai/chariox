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
}
impl SkillTurnAuthority {
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
}
