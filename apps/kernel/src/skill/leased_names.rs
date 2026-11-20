//! MP-08 / MP-10 / MP-11: a leased worker holds no catalog, so the home names the
//! skills a `$name` typed in the worker's native composer may select.
use super::{
    catalog::failure,
    selection::explicit_skill_names,
    v2::{SkillDelivery, SkillProjection, SkillState, SkillTransferManifest},
};
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The home's name resolution for one lease, sent with the packages it names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeasedSkillNames {
    pub catalog_revision: u64,
    pub skills: Vec<LeasedSkillName>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeasedSkillName {
    pub skill_id: String,
    pub name: String,
    pub digest: String,
}

impl LeasedSkillNames {
    /// The first row of a name wins, as in the home's own resolution. A row that is
    /// Off, or that the worker's harness provides, delivers nothing from the cache.
    pub fn from_projections(catalog_revision: u64, rows: &[SkillProjection]) -> Option<Self> {
        let mut seen = BTreeSet::new();
        let skills: Vec<_> = rows
            .iter()
            .filter(|row| seen.insert(row.skill.name.to_ascii_lowercase()))
            .filter(|row| {
                row.effective_state != SkillState::Off
                    && !matches!(row.delivery, SkillDelivery::ProvidedBy { .. })
            })
            .map(|row| LeasedSkillName {
                skill_id: row.skill.id.clone(),
                name: row.skill.name.clone(),
                digest: row.skill.digest.clone(),
            })
            .collect();
        (!skills.is_empty()).then_some(Self {
            catalog_revision,
            skills,
        })
    }

    /// The skills a typed prompt selects, in typing order. Dollar text that names
    /// nothing here is not an invocation.
    pub fn resolve(&self, prompt: &str) -> Vec<&LeasedSkillName> {
        explicit_skill_names(prompt)
            .iter()
            .filter_map(|typed| {
                self.skills
                    .iter()
                    .find(|skill| skill.name.eq_ignore_ascii_case(typed))
            })
            .collect()
    }

    /// Checked once where the view enters the worker: every name is a skill the same
    /// request transferred, so the cache can serve it.
    pub fn check_transferred(&self, manifest: &SkillTransferManifest) -> Result<(), DaemonError> {
        if self.catalog_revision != manifest.catalog_revision {
            return Err(failure(
                "skill names belong to a different catalog revision",
            ));
        }
        for skill in &self.skills {
            if !manifest
                .skills
                .iter()
                .any(|entry| entry.skill_id == skill.skill_id && entry.digest == skill.digest)
            {
                return Err(failure("skill name is not part of the skill transfer"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill::v2::{SkillScope, SkillTransferEntry};

    fn names(skills: &[(&str, &str)]) -> LeasedSkillNames {
        LeasedSkillNames {
            catalog_revision: 3,
            skills: skills
                .iter()
                .map(|(name, id)| LeasedSkillName {
                    skill_id: (*id).into(),
                    name: (*name).into(),
                    digest: "a".repeat(64),
                })
                .collect(),
        }
    }

    #[test]
    fn typed_names_resolve_like_the_home_in_typing_order() {
        let view = names(&[("review", "s1"), ("Deploy", "s2")]);
        let typed =
            view.resolve("run $deploy then $REVIEW? no: $review again, `$other`, $HOME, $9");
        assert_eq!(
            typed
                .iter()
                .map(|skill| skill.skill_id.as_str())
                .collect::<Vec<_>>(),
            ["s2", "s1"]
        );
        assert!(view.resolve("```\n$review\n```").is_empty());
    }

    #[test]
    fn a_name_outside_the_transfer_is_refused_at_the_worker_boundary() {
        let manifest = SkillTransferManifest {
            catalog_revision: 3,
            project_id: None,
            skills: vec![SkillTransferEntry {
                skill_id: "s1".into(),
                digest: "a".repeat(64),
                scope: SkillScope::Chariox,
                state: SkillState::Manual,
            }],
        };
        names(&[("review", "s1")])
            .check_transferred(&manifest)
            .unwrap();
        assert!(names(&[("review", "other")])
            .check_transferred(&manifest)
            .is_err());
        let mut stale = names(&[("review", "s1")]);
        stale.catalog_revision = 2;
        assert!(stale.check_transferred(&manifest).is_err());
    }
}
