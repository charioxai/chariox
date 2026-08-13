export * from "@chariox/kernel-client/ipc-requests"

export function listExternalProviderSessionsRequest(
  options: {
    provider?: string | null
    cursor?: string | null
    limit?: number | null
  } = {},
) {
  return {
    ListExternalProviderSessions: {
      provider: options.provider ?? null,
      cursor: options.cursor ?? null,
      limit: options.limit ?? null,
    },
  }
}

export function importExternalProviderSessionRequest(
  externalSessionId: string,
  options: {
    alias?: string | null
    provider?: string | null
    model?: string | null
    effort?: string | null
    worktreeId?: string | null
  } = {},
) {
  return {
    ImportExternalProviderSession: {
      external_session_id: externalSessionId,
      alias: options.alias ?? null,
      provider: options.provider ?? null,
      model: options.model ?? null,
      effort: options.effort ?? null,
      worktree_id: options.worktreeId ?? null,
    },
  }
}

export function importExternalProviderAgentRequest(
  sessionId: string,
  externalSessionId: string,
  options: {
    alias?: string | null
    provider?: string | null
    model?: string | null
    effort?: string | null
    focus?: boolean | null
  } = {},
) {
  return {
    ImportExternalProviderAgent: {
      session_id: sessionId,
      external_session_id: externalSessionId,
      alias: options.alias ?? null,
      provider: options.provider ?? null,
      model: options.model ?? null,
      effort: options.effort ?? null,
      focus: options.focus ?? null,
    },
  }
}

export function respondToInteractionRequest(
  sessionId: string,
  interactionId: string,
  choiceId: string,
  customReply?: string | null,
) {
  return {
    RespondToInteraction: {
      session_id: sessionId,
      interaction_id: interactionId,
      choice_id: choiceId,
      custom_reply: customReply ?? null,
    },
  }
}
