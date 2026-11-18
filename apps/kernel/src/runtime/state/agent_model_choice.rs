//! An agent always runs an explicit model and variant: the choice is made here, from the catalog
//! of the agent's account, the same way New Room preselects one. A model the person (or the
//! caller) names now must be one the account offers; a model the agent already runs stays; a
//! model the new account does not offer, or none at all, becomes that account's default; a
//! variant the model does not support becomes its default variant.

use std::collections::BTreeSet;

use crate::provider::{OpenCodeProviderCatalog, OpenCodeProviderInfo, OpenCodeProviderModel};

/// Effort names have a numeric order; provider-specific variants keep catalog order.
const EFFORT_ORDER: [&str; 8] = [
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];
const PREFERRED_VARIANT: &str = "high";

/// Where a requested model or variant comes from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// Not chosen by the caller: a model the account does not offer becomes its default.
    Inherited,
    /// Already in use (an alias, a withdrawn model, a Room default): it stays even when the
    /// catalog does not list it.
    Kept,
    /// The caller asks for it now: a model must be offered, a variant is taken as named.
    Named,
}

#[derive(Clone, Copy)]
pub(super) struct Requested<'a> {
    pub value: Option<&'a str>,
    pub source: Source,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct AgentModelChoice {
    pub model: String,
    pub effort: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ModelRefusal {
    NoModelOffered,
    /// The caller named a model the account's catalog does not list.
    NotOffered {
        wanted: String,
        available: Vec<String>,
    },
}

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
) -> Result<AgentModelChoice, ModelRefusal> {
    let offered = offered_models(catalog, provider);
    // `default` asks for the account's own default model, whoever asks.
    let wanted = model
        .value
        .map(str::trim)
        .filter(|value| !value.is_empty() && (*value != "default" || model.source == Source::Kept));
    let matched = wanted.and_then(|wanted| resolve_name(&offered, provider, wanted));
    let entry = match (matched, wanted) {
        (Some(entry), _) => Some(entry),
        (None, Some(_)) if model.source == Source::Named && offered.is_empty() => {
            return Err(ModelRefusal::NoModelOffered);
        }
        (None, Some(wanted)) if model.source == Source::Named => {
            return Err(ModelRefusal::NotOffered {
                wanted: wanted.to_string(),
                available: offered.iter().map(|entry| entry.id.clone()).collect(),
            });
        }
        (None, Some(_)) if model.source != Source::Inherited => None,
        _ => Some(default_entry(catalog, &offered).ok_or(ModelRefusal::NoModelOffered)?),
    };
    let model_id = match (entry, wanted) {
        (Some(entry), _) => entry.id.clone(),
        (None, Some(wanted)) => wanted.to_string(),
        (None, None) => return Err(ModelRefusal::NoModelOffered),
    };
    let requested_effort = effort
        .value
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let effort = match (requested_effort, effort.source != Source::Inherited, entry) {
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

/// Maps every name the provider's tool accepts to the one catalog entry it stands for: the id, the
/// id behind a provider qualifier of the account's family (`claude/…`, `codex/…`), a
/// name the catalog lists as an alias (`sonnet`), and an OpenCode model id without its provider.
fn resolve_name<'a, 'b>(
    offered: &'b [Offered<'a>],
    provider: &str,
    wanted: &str,
) -> Option<&'b Offered<'a>> {
    let unqualified = unqualified_name(provider, wanted);
    let names = [Some(wanted), unqualified];
    let exact = |name: &str| offered.iter().find(|entry| entry.id == name);
    let alias = |name: &str| {
        offered
            .iter()
            .find(|entry| entry.model.aliases.iter().any(|alias| alias == name))
    };
    names
        .iter()
        .flatten()
        .find_map(|name| exact(name))
        .or_else(|| names.iter().flatten().find_map(|name| alias(name)))
        .or_else(|| {
            let bare = unqualified.unwrap_or(wanted);
            (!bare.contains('/'))
                .then(|| offered.iter().find(|entry| entry.model.id == bare))
                .flatten()
        })
}

/// The name behind a provider qualifier: any provider of the account's family (`claude`,
/// `claude-p`, `claude-headless`, `codex`). An OpenCode id keeps its provider, which is part of the
/// model identity.
fn unqualified_name<'a>(provider: &str, wanted: &'a str) -> Option<&'a str> {
    let (qualifier, rest) = wanted.split_once('/')?;
    let family = crate::provider::canonical_provider_family(provider);
    (family.is_some()
        && family != Some("opencode")
        && crate::provider::canonical_provider_family(qualifier) == family
        && !rest.is_empty())
    .then_some(rest)
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
            aliases: Vec::new(),
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

    fn with_aliases(
        mut catalog: OpenCodeProviderCatalog,
        model: &str,
        aliases: &[&str],
    ) -> OpenCodeProviderCatalog {
        for provider in &mut catalog.all {
            if let Some(entry) = provider.models.get_mut(model) {
                entry.aliases = aliases.iter().map(|alias| alias.to_string()).collect();
            }
        }
        catalog
    }

    const NONE: Requested<'static> = Requested {
        value: None,
        source: Source::Inherited,
    };

    fn inherited(value: &str) -> Requested<'_> {
        Requested {
            value: Some(value),
            source: Source::Inherited,
        }
    }

    fn kept(value: &str) -> Requested<'_> {
        Requested {
            value: Some(value),
            source: Source::Kept,
        }
    }

    fn named(value: &str) -> Requested<'_> {
        Requested {
            value: Some(value),
            source: Source::Named,
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
    fn a_kept_model_and_a_named_variant_stay_even_when_unlisted() {
        let choice = choose(&catalog(), "codex", kept("future"), named("ultra")).unwrap();
        assert_eq!(choice.model, "future");
        assert_eq!(choice.effort.as_deref(), Some("ultra"));
    }

    #[test]
    fn a_named_model_the_account_does_not_list_is_refused_with_the_offered_ones() {
        assert_eq!(
            choose(&catalog(), "claude-p", named("haiku-5-5"), NONE),
            Err(ModelRefusal::NotOffered {
                wanted: "haiku-5-5".to_string(),
                available: vec!["claude-sonnet-5-5".to_string()],
            })
        );
    }

    #[test]
    fn a_named_model_the_account_lists_is_chosen() {
        let choice = choose(&catalog(), "claude-p", named("claude-sonnet-5-5"), NONE).unwrap();
        assert_eq!(choice.model, "claude-sonnet-5-5");
    }

    #[test]
    fn every_name_form_the_provider_accepts_resolves_to_the_catalog_id() {
        let catalog = with_aliases(catalog(), "claude-sonnet-5-5", &["sonnet"]);
        for wanted in [
            "claude-sonnet-5-5",
            "sonnet",
            "claude/claude-sonnet-5-5",
            "claude-p/claude-sonnet-5-5",
            "claude-headless/claude-sonnet-5-5",
            "claude/sonnet",
            " sonnet ",
        ] {
            for provider in ["claude", "claude-p"] {
                let choice = choose(&catalog, provider, named(wanted), NONE)
                    .unwrap_or_else(|_| panic!("{provider} {wanted}"));
                assert_eq!(choice.model, "claude-sonnet-5-5", "{provider} {wanted}");
            }
        }
        let catalog = with_aliases(catalog, "gpt-6-luna", &["picker-luna"]);
        for wanted in [
            "gpt-6-luna",
            "codex/gpt-6-luna",
            "picker-luna",
            "codex/picker-luna",
        ] {
            let choice = choose(&catalog, "codex", named(wanted), NONE).unwrap();
            assert_eq!(choice.model, "gpt-6-luna", "{wanted}");
        }
        let choice = choose(&catalog, "opencode", named("opencode-go/kimi"), NONE).unwrap();
        assert_eq!(choice.model, "opencode-go/kimi");
        let choice = choose(&catalog, "opencode", named("kimi"), NONE).unwrap();
        assert_eq!(choice.model, "opencode-go/kimi");
    }

    #[test]
    fn a_qualifier_of_another_provider_or_an_unknown_alias_is_still_refused() {
        let catalog = with_aliases(catalog(), "claude-sonnet-5-5", &["sonnet"]);
        for (provider, wanted) in [
            ("claude", "codex/claude-sonnet-5-5"),
            ("claude", "claude/opus"),
            ("claude", "gpt-6-luna"),
            ("codex", "claude/gpt-6-luna"),
            ("claude", "sonnet/"),
        ] {
            let Err(ModelRefusal::NotOffered {
                wanted: refused, ..
            }) = choose(&catalog, provider, named(wanted), NONE)
            else {
                panic!("{provider} {wanted} should be refused");
            };
            assert_eq!(refused, wanted);
        }
    }

    #[test]
    fn a_kept_alias_is_stored_as_the_id_it_stands_for() {
        let catalog = with_aliases(catalog(), "claude-sonnet-5-5", &["sonnet"]);
        let choice = choose(&catalog, "claude", kept("sonnet"), NONE).unwrap();
        assert_eq!(choice.model, "claude-sonnet-5-5");
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
            Err(ModelRefusal::NoModelOffered)
        );
    }
}
