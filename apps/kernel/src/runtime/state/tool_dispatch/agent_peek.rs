//! MP-08 / MP-10 / MP-11 peek for agents: the room's agents, under the room's trace rule, through
//! the same kernel projection the web and external agents read.
use super::*;
use crate::local::{PeekAgentTranscriptRequest, ReadAgentTranscriptEntryRequest};
use crate::transport::runtime_tools::{PeekAgentArgs, ReadAgentEntryArgs, RuntimeToolResult};

enum PeekCall {
    List(PeekAgentArgs),
    Read(ReadAgentEntryArgs),
}

fn peek_failure(message: impl Into<String>) -> RuntimeToolResult {
    RuntimeToolResult {
        ok: false,
        payload: serde_json::json!({ "error": message.into() }),
    }
}

impl KernelRuntimeState {
    pub(super) async fn handle_agent_peek_runtime_tool(
        &self,
        session: &crate::session::RuntimeSession,
        requester: &crate::agent::AgentInstance,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let call = if tool_name == crate::transport::runtime_tools::READ_AGENT_ENTRY_TOOL {
            serde_json::from_value(arguments).map(PeekCall::Read)
        } else {
            serde_json::from_value(arguments).map(PeekCall::List)
        };
        let call = match call {
            Ok(call) => call,
            Err(error) => {
                return Ok(peek_failure(format!(
                    "invalid {tool_name} arguments: {error}"
                )))
            }
        };
        let reference = match &call {
            PeekCall::List(args) => &args.agent,
            PeekCall::Read(args) => &args.agent,
        };
        let agents = self.session_agents(session.id());
        let target = match resolve_session_agent(&agents, reference) {
            Ok(target) => target.clone(),
            Err(message) => return Ok(peek_failure(message)),
        };
        if !self.can_view_agent_trace_owned_by(
            session,
            requester.owner_user_id(),
            target.owner_user_id(),
        ) {
            return Ok(peek_failure(format!(
                "{} is not yours and the room does not show its trace to you",
                target.id()
            )));
        }
        self.flush_session_provider_history(session.id()).await?;
        let store = self.owned.operational_history_store.clone();
        let import = target.external_provider_import().cloned();
        let payload = match call {
            PeekCall::List(args) => {
                let mut peek = crate::runtime::history_requests::execute_agent_transcript_peek(
                    store,
                    PeekAgentTranscriptRequest {
                        session_id: session.id().into(),
                        agent_id: target.id().into(),
                        limit: args.limit,
                        before: args.before,
                    },
                    import,
                    crate::session::unix_epoch_ms(),
                )
                .await?;
                self.protect_agent_peek(session.id(), &mut peek);
                serde_json::to_value(peek)
            }
            PeekCall::Read(args) => {
                let mut page =
                    crate::runtime::history_requests::execute_agent_transcript_entry_read(
                        store,
                        ReadAgentTranscriptEntryRequest {
                            session_id: session.id().into(),
                            agent_id: target.id().into(),
                            entry_id: args.entry_id,
                            offset: args.offset,
                            limit: args.limit,
                        },
                        import,
                    )
                    .await?;
                self.protect_agent_peek_page(session.id(), &mut page);
                serde_json::to_value(page)
            }
        };
        Ok(RuntimeToolResult {
            ok: true,
            payload: payload.map_err(|error| DaemonError::LocalTransport {
                operation: "runtime_tool.peek_agent",
                message: error.to_string(),
            })?,
        })
    }

    /// Peeks reach agents: Vault values a row holds never leave in its titles or pages.
    pub(crate) fn protect_agent_peek(
        &self,
        room: &str,
        peek: &mut crate::local::AgentTranscriptPeek,
    ) {
        let observations = &self.owned.room_secret_observations;
        for entry in &mut peek.entries {
            entry.title = observations.scrub_text_or_withhold(room, &entry.title);
        }
    }

    pub(crate) fn protect_agent_peek_page(
        &self,
        room: &str,
        page: &mut crate::local::AgentTranscriptEntryPage,
    ) {
        let observations = &self.owned.room_secret_observations;
        page.title = observations.scrub_text_or_withhold(room, &page.title);
        page.text = observations.scrub_text_or_withhold(room, &page.text);
    }
}
