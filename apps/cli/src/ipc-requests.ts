export * from "@arroba/kernel-client/ipc-requests"

export function respondToInteractionRequest(sessionId: string, interactionId: string, choiceId: string) {
  return {
    RespondToInteraction: {
      session_id: sessionId,
      interaction_id: interactionId,
      choice_id: choiceId,
    },
  }
}
