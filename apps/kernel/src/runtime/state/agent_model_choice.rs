//! An agent always runs an explicit model and variant: the choice is made here, from the catalog
//! of the agent's account, the same way New Room preselects one. A model the person (or the
//! caller) named stays; a model the new account does not offer, or none at all, becomes that
//! account's default; a variant the model does not support becomes its default variant.

use std::collections::BTreeSet;

use crate::provider::{OpenCodeProviderCatalog, OpenCodeProviderInfo, OpenCodeProviderModel};

/// Effort names have a numeric order; provider-specific variants keep catalog order.
const EFFORT_ORDER: [&str; 8] = [
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];
const PREFERRED_VARIANT: &str = "high";

#[derive(Clone, Copy)]
pub(super) struct Requested<'a> {
    pub value: Option<&'a str>,
    /// The caller named it; it is kept even when the catalog does not list it.
    pub explicit: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct AgentModelChoice {
    pub model: String,
    pub effort: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct NoModelOffered;

struct Offered<'a> {
    id: String,
    provider_id: &'a str,
    provider_name: &'a str,
    model: &'a OpenCodeProviderModel,
}

pub(super) fn choose(
    catalog: &OpenCodeProviderCatalog,
    provider: &str,
    model: Requested<'_>,
    effort: Requested<'_>,
) -> Result<AgentModelChoice, NoModelOffered> {
    let offered = offered_models(catalog, provider);
    let wanted = model.value.map(str::trim).filter(|value| !value.is_empty());
    let matched = wanted.and_then(|wanted| {
        offered.iter().find(|entry| entry.id == wanted).or_else(|| {
            (!wanted.contains('/'))
                .then(|| offered.iter().find(|entry| entry.model.id == wanted))
                .flatten()
        })
    });
    let entry = match (matched, wanted) {
        (Some(entry), _) => Some(entry),
        (None, Some(_)) if model.explicit => None,
        _ => Some(default_entry(catalog, &offered).ok_or(NoModelOffered)?),
    };
    let model_id = match (entry, wanted) {
        (Some(entry), _) => entry.id.clone(),
        (None, Some(wanted)) => wanted.to_string(),
        (None, None) => return Err(NoModelOffered),
    };
    let requested_effort = effort
        .value
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let effort = match (requested_effort, effort.explicit, entry) {
        (Some(value), true, _) => Some(value.to_string()),
        // A model the catalog does not list has no variants to check against.
        (value, _, None) => value.map(str::to_string),
        (value, _, Some(entry)) => {
            let variants = ordered_variants(entry.model);
            match value {
                Some(value) if variants.iter().any(|variant| variant == value) => {
                    Some(value.to_string())
                }
                _ => variants
                    .iter()
                    .find(|variant| variant.as_str() == PREFERRED_VARIANT)
                    .or_else(|| variants.first())
                    .cloned(),
            }
        }
    };
    Ok(AgentModelChoice {
        model: model_id,
        effort,
    })
}

fn offered_models<'a>(catalog: &'a OpenCodeProviderCatalog, provider: &str) -> Vec<Offered<'a>> {
    let connected: BTreeSet<&str> = catalog.connected.iter().map(String::as_str).collect();
    let family = crate::provider::canonical_provider_family(provider);
    let mut offered: Vec<Offered<'a>> = catalog
        .all
        .iter()
        .filter(|info| connected.contains(info.id.as_str()) && belongs(info, provider))
        .flat_map(|info| {
            info.models
                .values()
                .filter(|model| model.status != "deprecated")
                .map(move |model| Offered {
                    // OpenCode's provider prefix is part of the model identity (Zen versus Go).
                    id: if family == Some("opencode") {
                        format!("{}/{}", info.id, model.id)
                    } else {
                        model.id.clone()
                    },
                    provider_id: info.id.as_str(),
                    provider_name: info.name.as_str(),
                    model,
                })
        })
        .collect();
    offered.sort_by(|left, right| {
        let key = |entry: &Offered<'_>| {
            (
                entry.provider_name.to_lowercase(),
                entry.model.name.to_lowercase(),
            )
        };
        key(left).cmp(&key(right))
    });
    offered
}

fn belongs(info: &OpenCodeProviderInfo, provider: &str) -> bool {
    match (
        crate::provider::canonical_provider_family(provider),
        provider.trim(),
    ) {
        (Some("codex"), _) => info.id == "codex",
        (Some("claude"), "claude-headless") => info.id == "claude-headless",
        (Some("claude"), _) => info.id == "claude-p" || info.id == "claude",
        (Some("opencode"), _) => !matches!(
            info.id.as_str(),
            "codex" | "claude" | "claude-headless" | "claude-p"
        ),
        _ => false,
    }
}

/// The default the catalog names for the first provider that lists it, else the first model.
fn default_entry<'a, 'b>(
    catalog: &OpenCodeProviderCatalog,
    offered: &'b [Offered<'a>],
) -> Option<&'b Offered<'a>> {
    let mut providers: Vec<&str> = Vec::new();
    for entry in offered {
        if !providers.contains(&entry.provider_id) {
            providers.push(entry.provider_id);
        }
    }
    catalog
        .all
        .iter()
        .filter(|info| providers.contains(&info.id.as_str()))
        .find_map(|info| {
            let default = catalog.default.get(&info.id)?;
            offered
                .iter()
                .find(|entry| entry.provider_id == info.id && entry.model.id == *default)
        })
        .or_else(|| offered.first())
}

fn ordered_variants(model: &OpenCodeProviderModel) -> Vec<String> {
    let mut variants: Vec<String> = model.variants.keys().cloned().collect();
    if variants
        .iter()
        .all(|variant| EFFORT_ORDER.contains(&variant.as_str()))
    {
        variants.sort_by_key(|variant| {
            EFFORT_ORDER
                .iter()
                .position(|known| known == variant)
                .unwrap_or(usize::MAX)
        });
    }
    variants
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn model(id: &str, variants: &[&str]) -> OpenCodeProviderModel {
        OpenCodeProviderModel {
            id: id.to_string(),
            name: id.to_string(),
            status: "active".to_string(),
            limit: None,
            variants: variants
                .iter()
                .map(|variant| (variant.to_string(), serde_json::json!({})))
                .collect(),
        }
    }

    fn info(id: &str, models: &[(&str, &[&str])]) -> OpenCodeProviderInfo {
        OpenCodeProviderInfo {
            id: id.to_string(),
            name: id.to_string(),
            remote_machine_aliases: Vec::new(),
            models: models
                .iter()
                .map(|(model_id, variants)| (model_id.to_string(), model(model_id, variants)))
                .collect(),
        }
    }

    fn catalog() -> OpenCodeProviderCatalog {
        OpenCodeProviderCatalog {
            all: vec![
                info(
                    "codex",
                    &[
                        ("gpt-6-luna", &["low", "medium"]),
                        ("gpt-6.1-sol", &["low", "medium", "high", "xhigh"]),
                    ],
                ),
                info("claude-p", &[("claude-sonnet-5-5", &["low", "high"])]),
                info("opencode-go", &[("kimi", &[])]),
                info("opencode", &[("big-pickle", &[])]),
            ],
            default: BTreeMap::from([
                ("codex".to_string(), "gpt-6.1-sol".to_string()),
                ("opencode-go".to_string(), "kimi".to_string()),
            ]),
            connected: ["codex", "claude-p", "opencode-go", "opencode"]
                .map(str::to_string)
                .to_vec(),
        }
    }

    const NONE: Requested<'static> = Requested {
        value: None,
        explicit: false,
    };

    fn inherited(value: &str) -> Requested<'_> {
        Requested {
            value: Some(value),
            explicit: false,
        }
    }

    fn explicit(value: &str) -> Requested<'_> {
        Requested {
            value: Some(value),
            explicit: true,
        }
    }

    #[test]
    fn no_model_becomes_the_account_default_with_its_variant() {
        let choice = choose(&catalog(), "codex", NONE, NONE).unwrap();
        assert_eq!(choice.model, "gpt-6.1-sol");
        assert_eq!(choice.effort.as_deref(), Some("high"));
    }

    #[test]
    fn a_model_the_account_offers_is_kept_with_a_supported_variant() {
        let choice = choose(
            &catalog(),
            "codex",
            inherited("gpt-6-luna"),
            inherited("medium"),
        )
        .unwrap();
        assert_eq!(choice.model, "gpt-6-luna");
        assert_eq!(choice.effort.as_deref(), Some("medium"));
    }

    #[test]
    fn an_unsupported_variant_becomes_high_else_the_first_variant() {
        let choice = choose(
            &catalog(),
            "codex",
            inherited("gpt-6-luna"),
            inherited("xhigh"),
        )
        .unwrap();
        assert_eq!(choice.effort.as_deref(), Some("low"));
    }

    #[test]
    fn a_model_the_new_account_does_not_offer_becomes_its_default() {
        let choice = choose(
            &catalog(),
            "codex",
            inherited("claude-sonnet-5-5"),
            inherited("low"),
        )
        .unwrap();
        assert_eq!(choice.model, "gpt-6.1-sol");
        assert_eq!(choice.effort.as_deref(), Some("low"));
    }

    #[test]
    fn an_explicit_model_and_variant_stay_even_when_unlisted() {
        let choice = choose(&catalog(), "codex", explicit("future"), explicit("ultra")).unwrap();
        assert_eq!(choice.model, "future");
        assert_eq!(choice.effort.as_deref(), Some("ultra"));
    }

    #[test]
    fn opencode_keeps_its_provider_prefix_and_picks_a_connected_default() {
        let choice = choose(&catalog(), "opencode", NONE, NONE).unwrap();
        assert_eq!(choice.model, "opencode-go/kimi");
        assert_eq!(choice.effort, None);
    }

    #[test]
    fn claude_models_are_bare_ids() {
        let choice = choose(&catalog(), "claude-p", NONE, NONE).unwrap();
        assert_eq!(choice.model, "claude-sonnet-5-5");
        assert_eq!(choice.effort.as_deref(), Some("high"));
    }

    #[test]
    fn a_catalog_without_models_is_an_error() {
        assert_eq!(
            choose(&catalog(), "claude-headless", NONE, NONE),
            Err(NoModelOffered)
        );
    }
}
