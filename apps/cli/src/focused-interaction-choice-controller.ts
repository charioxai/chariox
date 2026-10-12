import type { RuntimeInteraction, RuntimeSession } from "./cli-types.js"
import type { FooterFlash } from "./footer-flash-controller.js"
import {
  appendInteractionCustomReply,
  deleteInteractionCustomReply,
  interactionCustomChoiceIndex,
  interactionCustomReplyPasteText,
  nextInteractionChoiceIndex,
  resolveInteractionChoiceKeyAction,
  resolveInteractionChoiceSubmission,
} from "@chariox/kernel-client/interaction-choice"

export type FocusedInteractionChoiceKeyEvent = {
  name: string
  sequence?: string
  eventType?: string
  ctrl?: boolean
  meta?: boolean
  alt?: boolean
  shift?: boolean
  preventDefault?: () => void
  stopPropagation?: () => void
}

export type FocusedInteractionChoicePasteEvent = {
  text: string
  /** The paste as the terminal sent it, before OpenTUI strips ANSI codes. */
  rawText?: string | null
  defaultPrevented?: boolean
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
  /** Provider login interactions: link actions and the kernel's result. */
  providerLogin?: {
    stripState: (interaction: RuntimeInteraction) => { view: { url: string | null } } | null
    codeSent: (interaction: RuntimeInteraction) => boolean
    openLink: (interaction: RuntimeInteraction) => Promise<void>
    copyLink: (interaction: RuntimeInteraction) => Promise<void>
  }
}

export type FocusedInteractionChoiceController = {
  submitChoice(choiceIndex?: number): Promise<boolean>
  cycleChoice(delta: number): boolean
  handleKey(event: FocusedInteractionChoiceKeyEvent): boolean
  handlePaste(event: FocusedInteractionChoicePasteEvent): boolean
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
    const providerLogin = Boolean(interaction.provider_login)
    if (submitDecision.action === "edit_custom") {
      deps.setCustomEditing(interaction.id, true)
      repaintInteractions()
      if (deps.providerLogin?.stripState(interaction)) deps.flashFooter("Paste the code from the provider's page first", "info")
      return true
    }
    deps.setSelectedIndex(interaction.id, submitDecision.selectedIndex)
    const submittedSecret = Boolean(
      interaction.custom_choice
        && submitDecision.customReply
        && interaction.custom_choice.input_kind === "secret",
    )
    if (submittedSecret) {
      deps.clearCustomReply(interaction.id)
      deps.setCustomEditing(interaction.id, false)
      repaintInteractions()
    }
    try {
      const session = await deps.respondToInteraction(
        deps.getSessionId(),
        interaction.id,
        submitDecision.choiceId,
        submitDecision.customReply,
      )
      deps.applySessionState(session)
      if (!submittedSecret) {
        deps.clearCustomReply(interaction.id)
        deps.setCustomEditing(interaction.id, false)
      }
      // A sent code is answered by the kernel's login status, not here.
      if (!(providerLogin && submitDecision.choiceId === interaction.custom_choice?.id && deps.providerLogin?.codeSent(interaction))) {
        deps.flashFooter(providerLogin
          ? submitDecision.choiceId === "cancel" ? "Cancelling the sign-in…" : "Sign-in continues; waiting for the kernel…"
          : "interaction answered", "info")
      }
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
    // O and C act on a login link while its code field is focused and empty.
    if (
      (event.name === "o" || event.name === "c") && !event.ctrl && !event.meta && !event.alt
      && deps.isCustomEditing(interaction.id) && !deps.getCustomReply(interaction.id)
      && deps.providerLogin?.stripState(interaction)?.view.url
    ) {
      event.preventDefault?.()
      event.stopPropagation?.()
      void (event.name === "o" ? deps.providerLogin.openLink(interaction) : deps.providerLogin.copyLink(interaction))
        .catch((error) => deps.flashFooter(formatError(error), "error"))
      return true
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

  // A paste while a custom reply is being typed belongs to that reply, never
  // to the prompt: a pasted vault passphrase must not appear in the prompt.
  // A selected secret custom choice takes the paste even before editing.
  const handlePaste = (event: FocusedInteractionChoicePasteEvent) => {
    const interaction = deps.getFocusedInteraction()
    const customChoice = interaction?.custom_choice
    if (!interaction || !customChoice || event.defaultPrevented) {
      return false
    }
    const customIndex = interactionCustomChoiceIndex(interaction)
    const secretSelected = customChoice.input_kind === "secret"
      && (deps.getSelectedIndex(interaction.id) ?? 0) === customIndex
    if (!deps.isCustomEditing(interaction.id) && !secretSelected) {
      return false
    }
    event.preventDefault?.()
    event.stopPropagation?.()
    const text = interactionCustomReplyPasteText(event.rawText ?? event.text)
    if (text === null) {
      deps.flashFooter("paste not added: a reply is one line without control characters", "error")
      return true
    }
    deps.setSelectedIndex(interaction.id, customIndex)
    deps.setCustomEditing(interaction.id, true)
    deps.setCustomReply(interaction.id, appendInteractionCustomReply({
      current: deps.getCustomReply(interaction.id),
      input: text,
      maxLength: customChoice.max_length,
    }))
    repaintInteractions()
    return true
  }

  return {
    submitChoice,
    cycleChoice,
    handleKey,
    handlePaste,
  }
}
