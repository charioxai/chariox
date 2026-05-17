import type { RuntimeInteraction, RuntimeSession } from "./cli-types.js"
import type { FooterFlash } from "./footer-flash-controller.js"
import {
  appendInteractionCustomReply,
  deleteInteractionCustomReply,
  interactionCustomChoiceIndex,
  nextInteractionChoiceIndex,
  resolveInteractionChoiceKeyAction,
  resolveInteractionChoiceSubmission,
} from "./interaction-choice-state.js"

export type FocusedInteractionChoiceKeyEvent = {
  name: string
  eventType?: string
  ctrl?: boolean
  meta?: boolean
  alt?: boolean
  preventDefault?: () => void
  stopPropagation?: () => void
}

export type FocusedInteractionChoiceControllerDeps = {
  getFocusedInteraction: () => RuntimeInteraction | null
  isAttached: () => boolean
  getSessionId: () => string
  getSelectedIndex: (interactionId: string) => number | undefined
  setSelectedIndex: (interactionId: string, index: number) => void
  getCustomReply: (interactionId: string) => string
  setCustomReply: (interactionId: string, reply: string) => void
  clearCustomReply: (interactionId: string) => void
  isCustomEditing: (interactionId: string) => boolean
  setCustomEditing: (interactionId: string, editing: boolean) => void
  renderAgentInteractions: () => void
  applyResponseLayout: () => void
  respondToInteraction: (
    sessionId: string,
    interactionId: string,
    choiceId: string,
    customReply: string | null,
  ) => Promise<RuntimeSession>
  applySessionState: (session: RuntimeSession) => void
  flashFooter: (message: string, tone: FooterFlash["tone"]) => void
  formatError?: (error: unknown) => string
}

export type FocusedInteractionChoiceController = {
  submitChoice(choiceIndex?: number): Promise<boolean>
  cycleChoice(delta: number): boolean
  handleKey(event: FocusedInteractionChoiceKeyEvent): boolean
}

export function createFocusedInteractionChoiceController(
  deps: FocusedInteractionChoiceControllerDeps,
): FocusedInteractionChoiceController {
  const formatError = deps.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))

  const repaintInteractions = () => {
    deps.renderAgentInteractions()
    deps.applyResponseLayout()
  }

  const submitChoice = async (choiceIndex?: number) => {
    const interaction = deps.getFocusedInteraction()
    if (!interaction || !deps.isAttached()) {
      return false
    }
    const submitDecision = resolveInteractionChoiceSubmission({
      interaction,
      requestedIndex: choiceIndex,
      selectedIndex: deps.getSelectedIndex(interaction.id),
      customReply: deps.getCustomReply(interaction.id),
    })
    if (submitDecision.action === "unavailable") {
      return false
    }
    if (submitDecision.action === "edit_custom") {
      deps.setCustomEditing(interaction.id, true)
      repaintInteractions()
      return true
    }
    deps.setSelectedIndex(interaction.id, submitDecision.selectedIndex)
    try {
      const session = await deps.respondToInteraction(
        deps.getSessionId(),
        interaction.id,
        submitDecision.choiceId,
        submitDecision.customReply,
      )
      deps.applySessionState(session)
      deps.clearCustomReply(interaction.id)
      deps.setCustomEditing(interaction.id, false)
      deps.flashFooter("interaction answered", "info")
      return true
    } catch (error) {
      deps.flashFooter(formatError(error), "error")
      return true
    }
  }

  const cycleChoice = (delta: number) => {
    const interaction = deps.getFocusedInteraction()
    if (!interaction) {
      return false
    }
    const currentIndex = deps.getSelectedIndex(interaction.id) ?? 0
    const nextIndex = nextInteractionChoiceIndex({ interaction, currentIndex, delta })
    if (nextIndex === null) {
      return false
    }
    deps.setSelectedIndex(interaction.id, nextIndex)
    if (interaction.custom_choice && nextIndex !== interactionCustomChoiceIndex(interaction)) {
      deps.setCustomEditing(interaction.id, false)
    }
    repaintInteractions()
    return true
  }

  const handleKey = (event: FocusedInteractionChoiceKeyEvent) => {
    const interaction = deps.getFocusedInteraction()
    if (!interaction || event.eventType === "release") {
      return false
    }
    const keyAction = resolveInteractionChoiceKeyAction({
      interaction,
      event,
      selectedIndex: deps.getSelectedIndex(interaction.id) ?? 0,
      customEditing: deps.isCustomEditing(interaction.id),
      customReply: deps.getCustomReply(interaction.id),
    })
    if (keyAction.action === "ignore") {
      return false
    }
    if (keyAction.consumeEvent) {
      event.preventDefault?.()
      event.stopPropagation?.()
    }
    if (keyAction.action === "handled") {
      return true
    }
    if (keyAction.action === "cancel_custom_edit") {
      deps.setCustomEditing(interaction.id, false)
      repaintInteractions()
      return true
    }
    if (keyAction.action === "delete_custom_reply") {
      deps.setCustomReply(interaction.id, deleteInteractionCustomReply(deps.getCustomReply(interaction.id)))
      repaintInteractions()
      return true
    }
    if (keyAction.action === "append_custom_reply") {
      deps.setCustomReply(interaction.id, appendInteractionCustomReply({
        current: deps.getCustomReply(interaction.id),
        input: keyAction.input,
        maxLength: interaction.custom_choice?.max_length,
      }))
      repaintInteractions()
      return true
    }
    if (keyAction.action === "cycle") {
      return cycleChoice(keyAction.delta)
    }
    if (keyAction.action === "begin_custom_edit") {
      deps.setSelectedIndex(interaction.id, keyAction.selectedIndex)
      deps.setCustomEditing(interaction.id, true)
      repaintInteractions()
      return true
    }
    if (keyAction.action === "submit") {
      void submitChoice(keyAction.choiceIndex)
      return true
    }
    return false
  }

  return {
    submitChoice,
    cycleChoice,
    handleKey,
  }
}
