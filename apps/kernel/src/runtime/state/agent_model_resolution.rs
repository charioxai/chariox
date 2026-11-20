//! Reads the account catalog an agent's model and variant are chosen from (see
//! `agent_model_choice`) for the places that create or reconfigure an agent.

use super::agent_model_choice::{choose, ModelRefusal, Requested, Source};
use super::agent_profile_owned_state::{AgentProfileSelections, ValidatedAgentProfile};
use super::*;

impl KernelRuntimeState {
    /// The model and variant a new agent runs. Providers without a catalog (the dev stub) keep what
    /// the caller sent. A model the caller names now (`Source::Named`) must be one the account
    /// offers, and a model list that cannot be read refuses it; a model already in use
    /// (`Source::Kept`) needs no catalog when the variant is named too.
    pub(crate) async fn resolve_new_agent_selection(
        &self,
        owner_user_id: &str,
        provider: &str,
        account_profile: Option<&str>,
        model: Option<String>,
        model_source: Source,
        effort: Option<String>,
    ) -> Result<(Option<String>, Option<String>), DaemonError> {
        let Some(family) = crate::provider::canonical_provider_family(provider) else {
            return Ok((model, effort));
        };
        let registry = self.provider_account_profile_registry();
        let account_selection = account_profile.unwrap_or("default");
        let account_owner = crate::account_profile::provider_account_authority_owner_for_profile(
            &self.owned.config_projection.snapshot(),
            registry,
            owner_user_id,
            family,
            account_selection,
        )?;
        let is_explicit_go =
            family == "opencode" && crate::account_profile::is_opencode_go_model(model.as_deref());
        if is_explicit_go {
            self.verify_opencode_go_account(&account_owner, account_selection)
                .await?;
        }
        if model.is_some() && effort.is_some() && model_source != Source::Named {
            return Ok((model, effort));
        }
        // An account the owner does not have is refused by the launch admission that follows, with
        // the action to take; this step only chooses among the models of an account that exists.
        let Ok(profile) = registry.get(&account_owner, provider, account_selection) else {
            return Ok((model, effort));
        };
        let profile = profile.profile_id;
        let model_source = if model.is_some() {
            model_source
        } else {
            Source::Inherited
        };
        let catalog = match self
            .read_account_catalog(owner_user_id, provider, family, &profile)
            .await
        {
            Ok(catalog) => catalog,
            // A model already in use stays; its variant can wait for a readable catalog.
            Err(_) if model_source == Source::Kept => return Ok((model, effort)),
            Err(error) => return Err(error),
        };
        let choice = choose(
            &catalog,
            provider,
            Requested {
                value: model.as_deref(),
                source: model_source,
            },
            Requested {
                value: effort.as_deref(),
                source: if effort.is_some() {
                    Source::Named
                } else {
                    Source::Inherited
                },
            },
        )
        .map_err(|refusal| model_refused(provider, &profile, refusal))?;
        if !is_explicit_go
            && family == "opencode"
            && crate::account_profile::is_opencode_go_model(Some(&choice.model))
        {
            self.verify_opencode_go_account(&account_owner, &profile)
                .await?;
        }
        Ok((Some(choice.model), choice.effort))
    }

    /// The defaults a new Room's first agent starts from, with the model and variant explicit.
    pub(crate) async fn resolve_new_session_defaults(
        &self,
        owner_user_id: &str,
        defaults: Option<crate::session::SessionAgentDefaults>,
    ) -> Result<Option<crate::session::SessionAgentDefaults>, DaemonError> {
        let mut defaults = defaults.unwrap_or_default();
        defaults.provider = crate::account_profile::resolve_placeholder_provider(
            &self.owned.config_projection.snapshot(),
            self.provider_account_profile_registry(),
            owner_user_id,
            &defaults.provider,
        );
        let (model, effort) = self
            .resolve_new_agent_selection(
                owner_user_id,
                &defaults.provider,
                defaults.account_profile.as_deref(),
                defaults.model.take(),
                Source::Named,
                defaults.effort.take(),
            )
            .await?;
        defaults.model = model;
        defaults.effort = effort;
        Ok(Some(defaults))
    }

    /// The selections of a profile update with the model and variant made explicit. A model the
    /// agent keeps stays when the target account offers it; otherwise it becomes that account's
    /// default.
    pub(super) async fn resolve_updated_agent_selection(
        &self,
        caller_user_id: &str,
        original: &crate::agent::AgentInstance,
        target: &ValidatedAgentProfile,
        mut selections: AgentProfileSelections,
    ) -> Result<AgentProfileSelections, DaemonError> {
        let Some(family) = crate::provider::canonical_provider_family(&target.provider) else {
            return Ok(selections);
        };
        let model_named = selections.model.is_some();
        let effort_named = matches!(selections.effort, Some(Some(_)));
        let family_changed = family
            != crate::provider::canonical_provider_family(original.provider()).unwrap_or_default();
        // The stored selector may be the alias `default`; compare the account it stands for.
        let account_owner = self.provider_account_authority_owner_user_id(caller_user_id);
        let original_account = self
            .provider_account_profile_registry()
            .get(
                &account_owner,
                original.provider(),
                original.provider_account_profile(),
            )
            .map_or_else(
                |_| original.provider_account_profile().to_string(),
                |profile| profile.profile_id,
            );
        let account_changed = family_changed || target.account_profile != original_account;
        let model_changed = target.model.as_deref() != original.model();
        let keeps_model = !account_changed && target.model.is_some();
        // Naming the model the agent already runs on its account asks for nothing new.
        let model_requested = model_named && (model_changed || account_changed);
        let settled = target.model.is_some() && target.effort.is_some();
        if (model_named && effort_named && !model_requested)
            || (settled && !account_changed && !model_changed && !model_named)
        {
            return Ok(selections);
        }
        let catalog = match self
            .read_account_catalog(
                caller_user_id,
                &target.provider,
                family,
                &target.account_profile,
            )
            .await
        {
            Ok(catalog) => catalog,
            // The agent keeps a model it already runs; only its variant waits for a readable catalog.
            Err(error) if keeps_model && !model_requested => {
                crate::logging::warn_with_fields(
                    "agent.model",
                    "the model list could not be read; the agent keeps what it has",
                    serde_json::json!({ "agent": original.id(), "error": error.to_string() }),
                );
                return Ok(selections);
            }
            Err(error) => return Err(error),
        };
        let choice = choose(
            &catalog,
            &target.provider,
            Requested {
                value: target.model.as_deref(),
                source: if model_requested {
                    Source::Named
                } else if keeps_model {
                    // A model the agent already runs on the same account stays even when the
                    // catalog does not list it (an alias or a withdrawn model).
                    Source::Kept
                } else {
                    Source::Inherited
                },
            },
            Requested {
                value: target.effort.as_deref(),
                source: if effort_named {
                    Source::Named
                } else {
                    Source::Inherited
                },
            },
        )
        .map_err(|refusal| model_refused(&target.provider, &target.account_profile, refusal))?;
        // Selections that already match the validated target stay as they are, so a true
        // no-op is still recognised as one.
        if target.model.as_deref() != Some(choice.model.as_str()) {
            selections.model = Some(choice.model);
        }
        if target.effort != choice.effort {
            selections.effort = Some(choice.effort);
        }
        Ok(selections)
    }

    async fn read_account_catalog(
        &self,
        owner_user_id: &str,
        provider: &str,
        family: &str,
        profile_id: &str,
    ) -> Result<crate::provider::OpenCodeProviderCatalog, DaemonError> {
        let projection = self
            .with_app_side_effect(|app| app.provider_catalog_projection_store())
            .await;
        let request = crate::local::GetProviderCatalogRequest {
            provider: Some(provider.to_string()),
            account_profiles: BTreeMap::from([(family.to_string(), profile_id.to_string())]),
            ..Default::default()
        };
        crate::runtime::provider_catalog_control::load_catalog(
            &projection,
            &self.owned.config_projection,
            &self.owned.provider_account_profiles,
            owner_user_id,
            request,
            true,
        )
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "resolve agent model",
            message: format!(
                "the model list of {provider} account `{profile_id}` could not be read: {error}. Check the account and retry"
            ),
        })
    }
}

/// The most model ids an error lists; a catalog such as OpenCode's can hold hundreds.
const LISTED_MODELS: usize = 12;

fn model_refused(provider: &str, profile_id: &str, refusal: ModelRefusal) -> DaemonError {
    let message = match refusal {
        ModelRefusal::NoModelOffered => format!(
            "{provider} account `{profile_id}` offers no models. Check the account and retry"
        ),
        ModelRefusal::NotOffered { wanted, available } => format!(
            "unknown model \"{wanted}\" for provider {provider}; use one of: {}{}",
            available
                .iter()
                .take(LISTED_MODELS)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", "),
            if available.len() > LISTED_MODELS {
                ", ..."
            } else {
                ""
            },
        ),
    };
    let message = if crate::provider::canonical_provider_family(provider) == Some("opencode") {
        format!("{message}. If this model needs an account, sign in to its provider in Provider Accounts")
    } else {
        message
    };
    DaemonError::LocalTransport {
        operation: "resolve agent model",
        message,
    }
}
