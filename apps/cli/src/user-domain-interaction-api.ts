import { answerUserDomainInteractionRequest } from "@chariox/kernel-client/ipc-requests"
import type { InteractionPasskeyProof } from "./ipc-requests.js"
import type { UserAppViewClient } from "./user-app-view-controller.js"

export async function answerUserDomainInteraction(client: UserAppViewClient, interactionId: string, choiceId: string, proof?: InteractionPasskeyProof) {
  const response = await client.send(answerUserDomainInteractionRequest({ interactionId, choiceId,
    ...(proof ? {passkey: proof.passkey} : {}),
    ...(proof?.rememberMinutes != null ? {passkeyRememberMinutes: proof.rememberMinutes} : {}) }))
  if (response?.UserDomainInteractionAnswered?.interaction_id !== interactionId) throw new Error("The kernel did not confirm this answer")
  return response
}
