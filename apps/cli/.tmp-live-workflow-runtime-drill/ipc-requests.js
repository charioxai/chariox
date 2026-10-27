export * from "@chariox/kernel-client/ipc-requests";
export function listExternalProviderSessionsRequest(options = {}) {
  return {
    ListExternalProviderSessions: {
      provider: options.provider ?? null,
      cursor: options.cursor ?? null,
      limit: options.limit ?? null
    }
  };
}
export function importExternalProviderSessionRequest(externalSessionId, options = {}) {
  return {
    ImportExternalProviderSession: {
      external_session_id: externalSessionId,
      alias: options.alias ?? null,
      provider: options.provider ?? null,
      model: options.model ?? null,
      effort: options.effort ?? null,
      worktree_id: options.worktreeId ?? null
    }
  };
}
export function importExternalProviderAgentRequest(sessionId, externalSessionId, options = {}) {
  return {
    ImportExternalProviderAgent: {
      session_id: sessionId,
      external_session_id: externalSessionId,
      alias: options.alias ?? null,
      provider: options.provider ?? null,
      model: options.model ?? null,
      effort: options.effort ?? null,
      focus: options.focus ?? null
    }
  };
}
export function respondToInteractionRequest(sessionId, interactionId, choiceId, customReply) {
  return {
    RespondToInteraction: {
      session_id: sessionId,
      interaction_id: interactionId,
      choice_id: choiceId,
      custom_reply: customReply ?? null
    }
  };
}