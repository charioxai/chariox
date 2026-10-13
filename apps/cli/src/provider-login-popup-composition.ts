import type { BoxRenderable, CliRenderer } from "@opentui/core"
import { providerLoginInteractionReplyRequest } from "@chariox/kernel-client/provider-login-projection"
import { createFocusedInteractionChoiceController } from "./focused-interaction-choice-controller.js"
import type { createInteractionChoiceStoreController } from "./interaction-choice-store-controller.js"
import { renderInteractionStrip } from "./interaction-strip-renderer.js"
import type { ProviderLoginInteractionController } from "./provider-login-interaction-controller.js"

/** MP-08/MP-11: Provider Accounts renders the same kernel login template and
 * uses the attached interaction's renderer and masked input controller. */
export function createProviderLoginPopupComposition(deps: {
  renderer: CliRenderer
  attached(): boolean
  login: ProviderLoginInteractionController
  choices: ReturnType<typeof createInteractionChoiceStoreController>
  send(request: ReturnType<typeof providerLoginInteractionReplyRequest>): Promise<unknown>
  flashFooter(message: string, tone: "info" | "error"): void
  render(): void
}) {
  let phase = ""
  let previousId: string | null = null
  const interaction = () => {
    const current = deps.attached() ? null : deps.login.sessionlessInteraction()
    const next = current ? JSON.stringify([current.id, current.title, current.custom_choice?.id]) : ""
    if (next !== phase) {
      if (previousId) {
        deps.choices.clearCustomReply(previousId)
        deps.choices.setCustomEditing(previousId, false)
      }
      phase = next
      previousId = current?.id ?? null
      if (current?.custom_choice) {
        deps.choices.setSelectedIndex(current.id, current.choices.length)
        deps.choices.setCustomEditing(current.id, true)
      }
    }
    return current
  }
  const input = createFocusedInteractionChoiceController({
    getFocusedInteraction: interaction,
    isAttached: () => false,
    getSessionId: () => "",
    getSelectedIndex: deps.choices.getSelectedIndex,
    setSelectedIndex: deps.choices.setSelectedIndex,
    getCustomReply: deps.choices.customReply,
    setCustomReply: deps.choices.setCustomReply,
    clearCustomReply: deps.choices.clearCustomReply,
    isCustomEditing: deps.choices.isCustomEditing,
    setCustomEditing: deps.choices.setCustomEditing,
    renderAgentInteractions: deps.render,
    applyResponseLayout: () => {},
    respondToInteraction: async () => { throw new Error("Provider login has no attached session") },
    applySessionState: () => {},
    flashFooter: deps.flashFooter,
    providerLogin: {
      ...deps.login,
      respondSessionless: async (current, choiceId, reply) => {
        await deps.send(providerLoginInteractionReplyRequest(current, choiceId, reply))
      },
    },
  })
  return {
    active: () => interaction() !== null,
    handleKey: input.handleKey,
    handlePaste: input.handlePaste,
    render(panel: BoxRenderable) {
      const current = interaction()
      if (!current) return
      renderInteractionStrip({
        renderer: deps.renderer, primaryBox: panel, auxiliaryBoxes: [], visibleAgents: [],
        maxAgentsPerScreen: 1, focusedAgentId: current.agent_id ?? null,
        activeInteractionForAgent: () => current,
        selectedChoiceIndex: deps.choices.selectedChoiceIndex,
        setSelectedChoiceIndex: deps.choices.setSelectedIndex,
        customReply: deps.choices.customReply,
        customEditing: deps.choices.isCustomEditing,
        queuedPromptStripItemsForAgent: () => [], selectedQueuedPromptIndexForAgent: () => -1,
        onQueuedPromptAction: () => {},
        providerLoginState: deps.login.stripState,
        onProviderLoginLinkClick: () => { void deps.login.openLink(current) },
      }, panel, { id: current.agent_id ?? "" } as never)
    },
    dispose() {
      if (previousId) deps.choices.clearCustomReply(previousId)
    },
  }
}
