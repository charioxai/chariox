//! MP-08 / MP-10 / MP-11: current-turn instruction tools share the home catalog and permission policy.
use super::*;
use crate::skill::v2::{SkillCatalogMutation, SkillsV2Request};
use crate::transport::runtime_tools::{
    ListSkillsArgs, LoadSkillArgs, ManageSkillsArgs, RuntimeToolResult,
};

fn failure(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime_tool.skills",
        message: message.into(),
    }
}
impl KernelRuntimeState {
    pub(super) async fn handle_skill_runtime_tool(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        self.ensure_skills_v2_catalog_ready()?;
        self.authorize_current_external_command()?;
        let origin_prompt_id;
        let request = match tool {
            crate::transport::runtime_tools::LIST_SKILLS_TOOL => {
                let args: ListSkillsArgs = serde_json::from_value(arguments)
                    .map_err(|_| failure("invalid list_skills arguments"))?;
                self.current_skill_tool_prompt(session.id(), agent.id(), &args.origin_prompt_id)?;
                origin_prompt_id = args.origin_prompt_id;
                SkillsV2Request::List {
                    session_id: session.id().into(),
                    agent_id: agent.id().into(),
                    cursor: args.cursor,
                    limit: args.limit.unwrap_or(16),
                }
            }
            crate::transport::runtime_tools::LOAD_SKILL_TOOL => {
                let args: LoadSkillArgs = serde_json::from_value(arguments)
                    .map_err(|_| failure("load_skill requires skill_id and the current origin_prompt_id; model-supplied explicit authority is not accepted"))?;
                self.current_skill_tool_prompt(session.id(), agent.id(), &args.origin_prompt_id)?;
                origin_prompt_id = args.origin_prompt_id;
                SkillsV2Request::Load {
                    session_id: session.id().into(),
                    agent_id: agent.id().into(),
                    skill_id: args.skill_id,
                }
            }
            crate::transport::runtime_tools::MANAGE_SKILLS_TOOL => {
                let args: ManageSkillsArgs = serde_json::from_value(arguments)
                    .map_err(|_| failure("invalid manage_skills arguments; preview the exact batch before publishing it"))?;
                match args {
                    ManageSkillsArgs::Preview {
                        mutation,
                        origin_prompt_id: current_origin,
                    } => {
                        self.current_skill_tool_prompt(session.id(), agent.id(), &current_origin)?;
                        self.ensure_skill_mutation_owner(&mutation, agent.owner_user_id())?;
                        origin_prompt_id = current_origin;
                        SkillsV2Request::Preview { mutation }
                    }
                    ManageSkillsArgs::Publish {
                        mutation,
                        review_receipt_id,
                        origin_prompt_id: current_origin,
                    } => {
                        self.current_skill_tool_prompt(session.id(), agent.id(), &current_origin)?;
                        self.ensure_skill_mutation_owner(&mutation, agent.owner_user_id())?;
                        // Verify the reviewed bytes before asking. The service verifies
                        // the same receipt/revision again at actual publication.
                        let service = self.skills_catalog_service()?;
                        let (receipt, affected) = service.reviewed_changes(
                            agent.owner_user_id(),
                            &mutation,
                            &review_receipt_id,
                        )?;
                        if !self
                            .approve_skill_mutation(session, agent, &mutation, &receipt, &affected)
                            .await?
                        {
                            return Ok(RuntimeToolResult {
                                ok: false,
                                payload: serde_json::json!({"published":false,"reason":"permission_denied"}),
                            });
                        }
                        self.authorize_current_external_command()?;
                        self.current_skill_tool_prompt(session.id(), agent.id(), &current_origin)?;
                        origin_prompt_id = current_origin;
                        SkillsV2Request::Mutate {
                            mutation,
                            review_receipt_id,
                        }
                    }
                }
            }
            _ => return Err(failure("unknown skill runtime tool")),
        };
        let response = self
            .execute_skills_catalog_request_with_origin(
                request,
                agent.owner_user_id().into(),
                Some(super::super::skills_turn_authority::SkillToolOrigin {
                    session_id: session.id().into(),
                    agent_id: agent.id().into(),
                    prompt_id: origin_prompt_id,
                }),
            )
            .await?;
        let crate::local::LocalDaemonResponse::SkillsV2 { result } = response else {
            return Err(failure("unexpected home catalog response"));
        };
        Ok(RuntimeToolResult {
            ok: true,
            payload: serde_json::to_value(result)
                .map_err(|_| failure("cannot encode skill runtime response"))?,
        })
    }

    pub(in crate::runtime::state) fn current_skill_tool_prompt(
        &self,
        session_id: &str,
        agent_id: &str,
        origin_prompt_id: &str,
    ) -> Result<crate::session::PromptQueueItem, DaemonError> {
        let session = self.owned.session_store.get_session(session_id)?;
        self.owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .filter(|prompt| {
                prompt.id() == origin_prompt_id
                    && prompt.status() == crate::session::PromptStatus::Running
            })
            .ok_or_else(|| {
                failure(
                    "skill tool belongs to a different or finished turn; use the current prompt_id",
                )
            })
    }

    async fn approve_skill_mutation(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        mutation: &SkillCatalogMutation,
        receipt: &crate::skill::v2::SkillReviewReceipt,
        affected: &[crate::skill::v2::SkillRecord],
    ) -> Result<bool, DaemonError> {
        if !crate::session::effective_agent_extension_registration_authority(session, Some(agent))
            .requires_approval()
        {
            return Ok(true);
        }
        let members = affected
            .iter()
            .map(|record| {
                format!(
                    "- {} [{}] sha256={} files={} default={:?} override={:?}",
                    record.name,
                    record.id,
                    record.digest,
                    record.files.len(),
                    record.default_state,
                    record.state_override
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let states = mutation
            .actions
            .iter()
            .filter_map(|action| match action {
                crate::skill::v2::SkillCatalogAction::SetState { skill_id, state } => Some(
                    format!("skill {skill_id}: {state:?} (None resets to inherited)"),
                ),
                crate::skill::v2::SkillCatalogAction::SetPackageState { package_id, state } => {
                    Some(format!(
                        "package {package_id}: {state:?} (None resets to inherited)"
                    ))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let interaction=crate::session::RuntimeInteraction::new(
            format!("skill-catalog-permission-{:032x}",rand::random::<u128>()),agent.id(),
            crate::session::RuntimeInteractionKind::Permission,
            crate::session::RuntimeInteractionLevel::Warning,
            Some("Skills catalog change".into()),
            format!("Allow agent `{}` to publish {} reviewed skill catalog action(s) in {:?} at revision {}?\nReview {} binds these exact files and source digests:\n{}\n{}\nInstructions and bundled files are installed without executing scripts or granting tools.",
                agent.agent_ref(),mutation.actions.len(),mutation.scope,mutation.expected_revision,receipt.id,members,states),
            vec![crate::session::RuntimeInteractionChoice::new("allow","Allow","allow",Some(crate::session::RuntimeInteractionChoiceStyle::Primary)),
                 crate::session::RuntimeInteractionChoice::new("deny","Deny","deny",Some(crate::session::RuntimeInteractionChoiceStyle::Danger))],
            None,None,None,
        );
        let resolution = self
            .create_runtime_interaction(session.id(), interaction)
            .await?
            .await
            .map_err(|_| failure("skill catalog approval ended without a decision"))?;
        self.authorize_current_external_command()?;
        Ok(resolution.choice_id.as_deref() == Some("allow"))
    }
}
