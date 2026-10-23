//! MP-08 / MP-10 / MP-11: adapters consume a kernel-pinned SkillTurnContext.
//! History retains the original prompt. Bodies use existing hidden channels;
//! native selections use harness invocation, never imported/copied packages.
use super::skill_inventory::NativeSkillInventory;
use crate::{
    error::DaemonError,
    skill::{
        selection::{explicit_skill_tokens, explicit_skill_tokens_with_names},
        v2::*,
    },
};

/// Kernel-owned instruction preparation before the existing provider mailbox.
/// Implementations retain the ordinary catalog and prompt owners, never credentials.
pub(crate) trait SkillPromptBridge: Send + Sync {
    fn acknowledge_prompt(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
    ) -> Result<(), DaemonError>;
    /// Claude's hook receipt: its additionalContext reached the official harness.
    fn acknowledge_hook_context(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
    ) -> Result<(), DaemonError>;
    fn acknowledge_tools(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        batch: &crate::provider::ProviderPromptSignalBatch,
    ) -> Result<(), DaemonError>;
    /// Skills typed in the native composer of a leased worker had no pending load
    /// there: the home validates each against its own catalog and records the load.
    fn acknowledge_typed_leased(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        prompt_id: &str,
        skills: &[TypedSkillLoad],
    ) -> Result<(), DaemonError>;
    /// Native composer turns ask the owning home for the current catalog and active loads.
    fn prepare_typed_leased(
        &self,
        _run: &crate::provider::RuntimeProviderRun,
        _context: &crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext,
        _prompt: &crate::session::PromptQueueItem,
        _lease_id: &str,
        _names: Option<&crate::skill::leased_names::LeasedSkillNames>,
    ) -> Result<ProviderSkillDelivery, DaemonError> {
        Err(failure(
            "the home skill preparation bridge is unavailable; reattach the native provider",
        ))
    }
    fn prepare(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        session_id: &str,
        agent_id: &str,
        prompt_id: &str,
        origin_prompt_id: &str,
        envelope: &crate::prompt_assembly::PromptEnvelope,
    ) -> Result<Option<ProviderSkillDelivery>, DaemonError>;
}

/// One home-pinned body or confirmed native invocation the worker delivered.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum TypedSkillLoad {
    Chariox {
        skill_id: String,
        digest: String,
        reason: SkillLoadReason,
    },
    Native {
        entry: Box<NativeSkillEntry>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSkillDelivery {
    pub visible_prompt: String,
    /// Codex developer context, Claude additionalContext, OpenCode system field.
    pub hidden_context: String,
    /// Only execution-validated native paths may enter Codex skill input items.
    pub codex_input: Vec<serde_json::Value>,
    /// Availability is never a receipt. Record loads after actual harness ACK.
    pub requires_native_load_ack: Vec<SkillSelection>,
    pub typed_leased_loads: Vec<TypedSkillLoad>,
    pub leased_names: Option<crate::skill::leased_names::LeasedSkillNames>,
}
fn failure(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "provider.skill_delivery",
        message: message.into(),
    }
}

pub fn prepare_skill_delivery(
    provider: &str,
    prompt: &str,
    context: &SkillTurnContext,
    inventory: &NativeSkillInventory,
    explicit_selections: &[SkillSelection],
) -> Result<ProviderSkillDelivery, DaemonError> {
    if !matches!(provider, "codex" | "claude" | "opencode") {
        return Err(failure("unsupported native skill provider"));
    }
    let mut hidden_context = context.metadata.clone();
    let mut native_names = Vec::new();
    let mut codex_input = Vec::new();
    for selection in &context.native_selections {
        let SkillSelection::Native {
            provider: selected_provider,
            name,
            ..
        } = selection
        else {
            return Err(failure(
                "native invocation requires a provider-bound selection",
            ));
        };
        if selected_provider != provider {
            return Err(failure(
                "native selection belongs to a different provider; refresh Skills",
            ));
        }
        let path = inventory.native_path(selection)?;
        if native_names.contains(name) {
            continue;
        }
        native_names.push(name.clone());
        if provider == "codex" {
            codex_input.push(serde_json::json!({"type":"skill", "name":name, "path":path}));
        }
        if provider == "opencode" {
            // This is an instruction to the official server's skill tool, not
            // a slash command or a generic tool-execution endpoint.
            hidden_context.push_str(&format!("\nThe user explicitly selected From OpenCode skill {}. Load it using the official skill tool with name {} before continuing. Native permissions still apply.\n", serde_json::to_string(name).unwrap(), serde_json::to_string(name).unwrap()));
        }
    }
    for skill in &context.loaded {
        if matches!(skill.delivery, SkillDelivery::ProvidedBy { .. }) {
            if !context.native_selections.iter().any(|selection| matches!(selection, SkillSelection::Native { name, .. } if name.eq_ignore_ascii_case(&skill.name))) {
                return Err(failure("provided-by skill has no validated native invocation"));
            }
            if !skill.body.is_empty() {
                return Err(failure("native duplicate cannot also carry a Chariox body"));
            }
            continue;
        }
        if skill.body.is_empty() || skill.root.is_empty() {
            return Err(failure(
                "selected Chariox skill instructions are missing; reinstall and retry",
            ));
        }
        // Match the core selector's body-budget accounting exactly.
        hidden_context.push_str(&format!(
            "\nSkill {} ({}) at {}:\n{}\n",
            skill.name, skill.digest, skill.root, skill.body
        ));
        hidden_context.extend(crate::skill::resolver::conflict_notice(skill));
    }
    let selected_names: Vec<_> = explicit_selections
        .iter()
        .filter_map(|selection| {
            let SkillSelection::Chariox { skill_id, name, .. } = selection else {
                return None;
            };
            context
                .loaded
                .iter()
                .any(|body| &body.skill_id == skill_id && body.name.eq_ignore_ascii_case(name))
                .then_some(name.as_str())
        })
        .collect();
    let tokens = explicit_skill_tokens_with_names(prompt, &selected_names);
    let mut visible_prompt = String::new();
    let mut start = 0;
    for token in tokens {
        visible_prompt.push_str(&prompt[start..token.range.start]);
        if context.loaded.iter().any(|skill| {
            skill.name.eq_ignore_ascii_case(&token.name)
                && !matches!(skill.delivery, SkillDelivery::ProvidedBy { .. })
        }) {
            visible_prompt.push_str(&format!("[Chariox skill {}]", token.name));
        } else {
            visible_prompt.push_str(&prompt[token.range.clone()]);
        }
        start = token.range.end;
    }
    visible_prompt.push_str(&prompt[start..]);
    if provider == "claude" && !native_names.is_empty() {
        if native_names.len() != 1 {
            return Err(failure("Claude supports one explicit native skill command per turn; select one From Claude entry"));
        }
        let name = &native_names[0];
        // Native command execution belongs to Claude's normal harness. Remove
        // only admitted invocation tokens for the chosen native command.
        let mut task = String::new();
        let mut start = 0;
        for token in explicit_skill_tokens(&visible_prompt) {
            task.push_str(&visible_prompt[start..token.range.start]);
            if token.name != *name {
                task.push_str(&visible_prompt[token.range.clone()]);
            }
            start = token.range.end;
        }
        task.push_str(&visible_prompt[start..]);
        visible_prompt = format!("/{name} {}", task.trim());
    }
    Ok(ProviderSkillDelivery {
        visible_prompt,
        hidden_context,
        codex_input,
        requires_native_load_ack: context.native_selections.clone(),
        typed_leased_loads: Vec::new(),
        leased_names: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_assembly::{PromptEnvelope, PromptManifest};
    use crate::provider::skill_inventory::SkillInventoryBinding;
    use std::collections::BTreeMap;
    fn inventory() -> NativeSkillInventory {
        NativeSkillInventory::from_candidates(
            &SkillInventoryBinding {
                provider: "codex".into(),
                provider_version: "pinned-test".into(),
                account_profile: "selected".into(),
                execution_id: "execution".into(),
                cwd: "/actual".into(),
                environment: BTreeMap::new(),
                project_id: None,
                catalog_revision: 1,
                source_revision: 1,
            },
            vec![],
        )
        .unwrap()
    }
    fn context(name: &str) -> SkillTurnContext {
        SkillTurnContext {
            catalog_revision: 1,
            context_generation: "generation".into(),
            metadata: "current metadata".into(),
            metadata_tokens_upper_bound: 16,
            metadata_budget_tokens: 100,
            omitted_auto_count: 0,
            native_selections: vec![],
            loaded: vec![SkillResolvedBody {
                skill_id: "skill-1".into(),
                name: name.into(),
                digest: "a".repeat(64),
                root: "/immutable/skill-1/digest".into(),
                body: "selected instructions".into(),
                files: vec![],
                reason: SkillLoadReason::Explicit,
                delivery: SkillDelivery::Chariox,
            }],
        }
    }
    #[test]
    fn chariox_selection_is_a_visible_label_and_hidden_body_for_every_provider() {
        for provider in ["codex", "claude", "opencode"] {
            let delivery = prepare_skill_delivery(
                provider,
                "$review inspect `$review` and $HOME",
                &context("review"),
                &inventory(),
                &[],
            )
            .unwrap();
            assert_eq!(
                delivery.visible_prompt,
                "[Chariox skill review] inspect `$review` and $HOME"
            );
            assert!(delivery.hidden_context.contains("selected instructions"));
            assert!(delivery.codex_input.is_empty());
            assert!(!delivery.visible_prompt.contains("selected instructions"));
        }
    }
    #[test]
    fn a_delivered_body_with_a_different_native_namesake_says_which_instructions_win() {
        for (provider, notice) in [("claude", true), ("opencode", true), ("codex", false)] {
            let mut context = context("review");
            context.loaded[0].delivery = SkillDelivery::Conflict {
                provider: provider.into(),
                entry_id: "native-review".into(),
                native_digest: "b".repeat(64),
            };
            let delivery =
                prepare_skill_delivery(provider, "$review", &context, &inventory(), &[]).unwrap();
            let (body, after) = delivery
                .hidden_context
                .split_once("selected instructions")
                .unwrap();
            assert!(body.contains("review"));
            assert_eq!(
                after.contains("A different provider-native skill named review exists"),
                notice,
                "{provider}: {after:?}"
            );
        }
        let delivery =
            prepare_skill_delivery("claude", "$review", &context("review"), &inventory(), &[])
                .unwrap();
        assert!(!delivery
            .hidden_context
            .contains("different provider-native"));
    }
    #[test]
    fn explicit_uppercase_chip_can_resolve_without_rewriting_shell_variables_in_code() {
        let selection = SkillSelection::Chariox {
            skill_id: "skill-1".into(),
            name: "UPPER".into(),
            catalog_revision: 1,
        };
        let delivery = prepare_skill_delivery(
            "codex",
            "$UPPER inspect `$UPPER` and $HOME",
            &context("UPPER"),
            &inventory(),
            &[selection],
        )
        .unwrap();
        assert_eq!(
            delivery.visible_prompt,
            "[Chariox skill UPPER] inspect `$UPPER` and $HOME"
        );
    }
    #[test]
    fn selected_instruction_failure_cannot_turn_into_an_empty_successful_turn() {
        let mut context = context("review");
        context.loaded[0].body.clear();
        assert!(prepare_skill_delivery("claude", "$review", &context, &inventory(), &[]).is_err());
    }
    #[test]
    fn native_provider_paths_are_never_deserializable_or_persisted_in_prompt_envelopes() {
        let delivery =
            prepare_skill_delivery("codex", "$review", &context("review"), &inventory(), &[])
                .unwrap();
        let envelope = PromptEnvelope::new(
            "$review",
            "existing system context",
            vec![],
            PromptManifest::default(),
        )
        .with_skill_delivery(delivery);
        let mut value = serde_json::to_value(&envelope).unwrap();
        assert!(value.get("skill_delivery").is_none());
        value["skill_delivery"] = serde_json::json!({"codex_input":[{"type":"skill","name":"forged","path":"/untrusted"}]});
        let decoded: PromptEnvelope = serde_json::from_value(value).unwrap();
        assert!(decoded.skill_delivery.is_none());
        assert!(decoded
            .hidden_system_context
            .contains("existing system context"));
        assert!(decoded
            .hidden_system_context
            .contains("selected instructions"));
    }
}
