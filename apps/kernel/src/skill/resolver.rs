//! MP-08 / MP-10 / MP-11: state resolution, provider dedupe and bounded discovery.
use super::{
    catalog::{failure, SkillCatalogSet},
    v2::*,
};
use crate::error::DaemonError;

/// When neither the run nor its model names a window, capacity is the smallest
/// window of any supported provider model (128K), never a larger guess.
pub const UNKNOWN_MODEL_CONTEXT_FLOOR: u64 = 128_000;

/// Bytes of carried text per token: conservative, since code and paths
/// tokenize densely.
pub const BYTES_PER_TOKEN: u64 = 3;

/// Context left for selected skill bodies after what the turn already carries.
pub(crate) fn remaining_turn_budget(
    context_window: Option<u64>,
    context_tokens: Option<u64>,
    carried_bytes: u64,
) -> u64 {
    context_window
        .unwrap_or(UNKNOWN_MODEL_CONTEXT_FLOOR)
        .saturating_sub(context_tokens.unwrap_or(0))
        .saturating_sub(carried_bytes.div_ceil(BYTES_PER_TOKEN))
}

pub fn effective_state(skill: &SkillRecord, project_override: Option<SkillState>) -> SkillState {
    project_override
        .or(skill.state_override)
        .unwrap_or(skill.default_state)
}

pub fn projections(
    catalogs: &SkillCatalogSet,
    project_id: Option<&str>,
    native: &[NativeSkillEntry],
) -> Vec<SkillProjection> {
    let project_scope = project_id.map(|id| SkillScope::Project {
        project_id: id.into(),
    });
    let project = project_scope
        .as_ref()
        .and_then(|scope| catalogs.catalog(scope));
    let mut rows = Vec::new();
    for catalog in &catalogs.catalogs {
        if catalog.scope != SkillScope::Chariox && Some(&catalog.scope) != project_scope.as_ref() {
            continue;
        }
        for skill in &catalog.skills {
            let project_override = if skill.scope == SkillScope::Chariox {
                project
                    .and_then(|c| c.chariox_overrides.get(&skill.id))
                    .copied()
            } else {
                None
            };
            let state = effective_state(skill, project_override);
            let mut delivery = SkillDelivery::Chariox;
            let mut also_in_provider = None;
            for entry in native {
                let SkillSelection::Native {
                    entry_id,
                    name,
                    provider,
                    ..
                } = &entry.selection
                else {
                    continue;
                };
                if !name.eq_ignore_ascii_case(&skill.name)
                    || entry.exposure == NativeSkillExposure::Disabled
                {
                    continue;
                }
                if entry.exposure != NativeSkillExposure::Confirmed
                    || !entry.complete_manifest
                    || entry.digest.is_none()
                {
                    delivery = SkillDelivery::Unverified {
                        provider: provider.clone(),
                        entry_id: entry_id.clone(),
                    };
                } else if entry.digest.as_ref() != Some(&skill.digest) {
                    delivery = SkillDelivery::Conflict {
                        provider: provider.clone(),
                        entry_id: entry_id.clone(),
                        native_digest: entry.digest.clone().unwrap(),
                    };
                } else {
                    if state != SkillState::Off {
                        also_in_provider = Some(provider.clone());
                    }
                    if entry.default_state == state && state != SkillState::Off {
                        delivery = SkillDelivery::ProvidedBy {
                            provider: provider.clone(),
                            entry_id: entry_id.clone(),
                        };
                    }
                }
                break;
            }
            rows.push(SkillProjection {
                skill: skill.clone(),
                effective_state: state,
                project_override,
                delivery,
                also_in_provider,
            });
        }
    }
    rows.sort_by(|a, b| {
        a.skill
            .name
            .to_ascii_lowercase()
            .cmp(&b.skill.name.to_ascii_lowercase())
            .then(a.skill.id.cmp(&b.skill.id))
    });
    rows
}

/// Equal complete native content folds into the visible Chariox picker row.
/// Independent native state remains visible in Skills details; it must not add
/// a second picker row or pretend that Chariox Off disabled the native copy.
pub fn picker_native_entries(
    rows: &[SkillProjection],
    native: &[NativeSkillEntry],
) -> Vec<NativeSkillEntry> {
    native
        .iter()
        .filter(|entry| entry.exposure != NativeSkillExposure::Disabled)
        .filter(|entry| {
            let SkillSelection::Native { name, provider, .. } = &entry.selection else {
                return false;
            };
            !rows.iter().any(|row| {
                row.effective_state != SkillState::Off
                    && row.also_in_provider.as_ref() == Some(provider)
                    && row.skill.name.eq_ignore_ascii_case(name)
                    && entry.digest.as_ref() == Some(&row.skill.digest)
            })
        })
        .cloned()
        .collect()
}

/// A typed `$name` chip still names the same skill at the revision it was picked from,
/// and that skill is not Off.
pub fn admit_chariox_selection<'a>(
    rows: &'a [SkillProjection],
    revision: u64,
    skill_id: &str,
    name: &str,
    catalog_revision: u64,
) -> Result<&'a SkillProjection, DaemonError> {
    if catalog_revision != revision {
        return Err(failure(
            "selected skill catalog changed; refresh and reselect",
        ));
    }
    let row = resolve_name(rows, name)?;
    if row.skill.id != skill_id {
        return Err(failure("selected skill was replaced; reselect"));
    }
    Ok(row)
}

/// A queued `$name` chip was admitted against the catalog it was picked from (`picked`).
/// When its turn starts on a later revision, it still names the same, unchanged skill
/// that is not Off, or the turn ends with the change and what to do next.
pub fn admit_queued_chariox_selection<'a>(
    picked: &[SkillProjection],
    current: &'a [SkillProjection],
    skill_id: &str,
    name: &str,
) -> Result<&'a SkillProjection, DaemonError> {
    let shown: String = name.chars().take(128).collect();
    let after = |change: &str, next: &str| {
        failure(format!(
            "skill `{shown}` {change} after this prompt was queued; {next}"
        ))
    };
    let was = picked
        .iter()
        .find(|row| row.skill.id == skill_id)
        .ok_or_else(|| failure("selected skill catalog changed; refresh and reselect"))?;
    let Some(row) = current.iter().find(|row| row.skill.id == skill_id) else {
        return Err(
            if current
                .iter()
                .any(|row| row.skill.name.eq_ignore_ascii_case(&was.skill.name))
            {
                after("was replaced", "resend the prompt to use the new one")
            } else {
                after("was removed", "reinstall it or resend without it")
            },
        );
    };
    if row.effective_state == SkillState::Off {
        return Err(after(
            "was turned off",
            "turn it back on in Skills or resend without it",
        ));
    }
    if row.skill.digest != was.skill.digest {
        return Err(after(
            "changed",
            "resend the prompt to use the current version",
        ));
    }
    Ok(row)
}

pub fn resolve_name<'a>(
    rows: &'a [SkillProjection],
    name: &str,
) -> Result<&'a SkillProjection, DaemonError> {
    let row = rows
        .iter()
        .find(|row| row.skill.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| {
            failure(format!(
                "unknown skill `${}`; open Skills to install or select a current name",
                name.chars().take(128).collect::<String>()
            ))
        })?;
    if row.effective_state == SkillState::Off {
        return Err(failure(format!(
            "skill `${}` is Off; enable it in Skills before selecting it",
            row.skill.name
        )));
    }
    Ok(row)
}

/// Discovery lists skills by name, which the catalog keeps unique regardless of
/// case; list_skills also returns ids. Either finds the same skill.
pub fn admit_load<'a>(
    rows: &'a [SkillProjection],
    name_or_id: &str,
    explicit_skill_ids: &[String],
) -> Result<&'a SkillProjection, DaemonError> {
    let row = rows
        .iter()
        .find(|row| row.skill.id == name_or_id)
        .or_else(|| {
            rows.iter()
                .find(|row| row.skill.name.eq_ignore_ascii_case(name_or_id))
        })
        .ok_or_else(|| {
            failure("unknown skill; pass a listed skill name or an id returned by list_skills (or the skill was removed; refresh Skills)")
        })?;
    match row.effective_state {
        SkillState::Off => Err(failure("skill is Off; enable it in Skills before loading")),
        SkillState::Manual if !explicit_skill_ids.contains(&row.skill.id) => Err(failure(
            "Manual skills require an explicit user $name selection",
        )),
        _ => Ok(row),
    }
}

/// A delivered Chariox body can be contradicted by a different same-named native
/// copy that the model reaches through its harness's own skill tool (Claude,
/// OpenCode). One line settles which instructions win.
pub fn conflict_notice(body: &SkillResolvedBody) -> Option<String> {
    match &body.delivery {
        SkillDelivery::Conflict { provider, .. } if provider != "codex" => Some(format!(
            "\nA different provider-native skill named {} exists; use only the instructions above for it.\n",
            body.name
        )),
        _ => None,
    }
}

pub fn validate_native_selection<'a>(
    selection: &SkillSelection,
    native: &'a [NativeSkillEntry],
    provider: &str,
    account: &str,
    execution: &str,
) -> Result<&'a NativeSkillEntry, DaemonError> {
    let SkillSelection::Native {
        provider: selected_provider,
        account_profile,
        execution_id,
        ..
    } = selection
    else {
        return Err(failure("expected a provider-bound native selection"));
    };
    if selected_provider != provider || account_profile != account || execution_id != execution {
        return Err(failure(
            "focused provider, account or execution place changed; reselect From Provider",
        ));
    }
    native
        .iter()
        .find(|entry| {
            &entry.selection == selection && entry.exposure == NativeSkillExposure::Confirmed
        })
        .ok_or_else(|| {
            failure(
                "native skill inventory changed or exposure is unverified; refresh and reselect",
            )
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillMetadataContext {
    pub text: String,
    pub tokens_upper_bound: u64,
    pub budget_tokens: u64,
    pub omitted_count: usize,
}

pub fn metadata_context(
    rows: &[SkillProjection],
    context_capacity: Option<u64>,
    budget_percent: f64,
) -> Result<SkillMetadataContext, DaemonError> {
    metadata_context_at(rows, context_capacity, budget_percent, None)
}

/// A leased worker has no Chariox tools: each line names the verified SKILL.md
/// the home transferred to that worker instead of pointing at list/load_skill.
pub fn metadata_context_at(
    rows: &[SkillProjection],
    context_capacity: Option<u64>,
    budget_percent: f64,
    worker_roots: Option<&std::collections::BTreeMap<String, String>>,
) -> Result<SkillMetadataContext, DaemonError> {
    if !budget_percent.is_finite() || budget_percent <= 0.0 || budget_percent > 100.0 {
        return Err(failure(
            "skills.metadata_budget_percent must be finite, greater than 0 and at most 100",
        ));
    }
    let capacity = context_capacity.unwrap_or(UNKNOWN_MODEL_CONTEXT_FLOOR);
    let budget = ((capacity as f64) * budget_percent / 100.0).floor() as u64;
    let auto: Vec<_> = rows
        .iter()
        .filter(|row| {
            row.effective_state == SkillState::Auto
                && !matches!(row.delivery, SkillDelivery::ProvidedBy { .. })
        })
        .collect();
    if auto.is_empty() {
        return Ok(SkillMetadataContext {
            text: String::new(),
            tokens_upper_bound: 0,
            budget_tokens: budget,
            omitted_count: 0,
        });
    }
    let mut text = match worker_roots {
        Some(_) => "Skills: read a skill's SKILL.md before using it.\n",
        None => "Skills: load a matching skill by name with chariox.load_skill before you act.\n",
    }
    .to_string();
    if text.len() as u64 > budget {
        return Err(failure("skill metadata budget is too small for discovery; increase skills.metadata_budget_percent"));
    }
    let entries: Vec<_> = auto
        .iter()
        .map(|row| {
            let description = row
                .skill
                .short_description
                .as_ref()
                .unwrap_or(&row.skill.description);
            let prefix = format!("{}:", row.skill.name);
            let location = worker_roots
                .and_then(|roots| roots.get(&row.skill.id))
                .map(|root| format!(" (SKILL.md at {root}/SKILL.md)"))
                .unwrap_or_default();
            (prefix, location, description.as_str())
        })
        .collect();
    // Every listed skill gets its name before any description words, so one long
    // description cannot crowd later skills out of discovery.
    let budget_bytes = budget as usize;
    let named = |(prefix, location, _): &(String, String, &str)| prefix.len() + location.len() + 1;
    let note = |omitted: usize| match (omitted, worker_roots) {
        (0, _) => String::new(),
        (_, Some(_)) => format!("{omitted} more Auto skills are not listed here.\n"),
        (_, None) => format!(
            "{omitted} more Auto skills are not listed here; \
             page through them with chariox.list_skills.\n"
        ),
    };
    let mut listed = entries.len();
    let mut used = text.len() + entries.iter().map(named).sum::<usize>();
    while listed > 0 && used + note(entries.len() - listed).len() > budget_bytes {
        listed -= 1;
        used -= named(&entries[listed]);
    }
    let omitted = entries.len() - listed;
    let note = note(omitted);
    // A budget too small for the note still lists what fits, as before.
    let mut spare = budget_bytes.saturating_sub(used + note.len());
    // Whole descriptions go first, shortest first, while they fit: a concise
    // description beats a few words cut from each long one. The rest share
    // what remains.
    let full = |description: &str| {
        description
            .split_whitespace()
            .map(|word| word.len() + 1)
            .sum::<usize>()
    };
    let mut order: Vec<usize> = (0..listed).collect();
    order.sort_by_key(|&index| full(entries[index].2));
    let mut lines = vec![String::new(); listed];
    for (filled, &index) in order.iter().enumerate() {
        let (prefix, location, description) = &entries[index];
        let share = match full(description) {
            whole if whole <= spare => whole,
            _ => spare / (listed - filled),
        };
        let mut line = prefix.clone();
        for word in description.split_whitespace() {
            if line.len() - prefix.len() + word.len() + 1 > share {
                break;
            }
            line.push(' ');
            line.push_str(word);
        }
        spare -= line.len() - prefix.len();
        line.push_str(location);
        lines[index] = line;
    }
    for line in lines {
        text.push_str(&line);
        text.push('\n');
    }
    if used + note.len() <= budget_bytes {
        text.push_str(&note);
    }
    Ok(SkillMetadataContext {
        tokens_upper_bound: text.len() as u64,
        text,
        budget_tokens: budget,
        omitted_count: omitted,
    })
}

pub fn discovery_page(
    rows: &[SkillProjection],
    revision: u64,
    cursor: Option<&str>,
    limit: u32,
) -> Result<(Vec<SkillProjection>, Option<String>), DaemonError> {
    let offset = match cursor {
        None => 0,
        Some(cursor) => {
            let (r, n) = cursor
                .split_once(':')
                .ok_or_else(|| failure("invalid skill discovery cursor"))?;
            if r.parse::<u64>().ok() != Some(revision) {
                return Err(failure("skill catalog changed; restart discovery"));
            }
            n.parse::<usize>()
                .map_err(|_| failure("invalid skill discovery cursor"))?
        }
    };
    let auto: Vec<_> = rows
        .iter()
        .filter(|row| {
            row.effective_state == SkillState::Auto
                && !matches!(row.delivery, SkillDelivery::ProvidedBy { .. })
        })
        .collect();
    if offset > auto.len() {
        return Err(failure("invalid skill discovery cursor"));
    }
    let end = offset
        .saturating_add(limit.clamp(1, 100) as usize)
        .min(auto.len());
    let next = (end < auto.len()).then(|| format!("{revision}:{end}"));
    Ok((
        auto[offset..end].iter().map(|row| (*row).clone()).collect(),
        next,
    ))
}
