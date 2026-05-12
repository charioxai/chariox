export * from "@arroba/kernel-client/ipc-requests";
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