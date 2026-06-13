import type { RuntimeInteraction } from "./cli-types.js"

export type InteractionChoiceSubmissionDecision = {
  action: "submit"
  selectedIndex: number
  choiceId: string
  customReply: string | null
} | {
  action: "edit_custom"
} | {
  action: "unavailable"
}

export type InteractionChoiceKeyEvent = {
  name: string
  eventType?: string
  ctrl?: boolean
  meta?: boolean
  alt?: boolean
}

export type InteractionChoiceKeyAction = {
  action: "ignore"
} | {
  action: "handled"
  consumeEvent: boolean
} | {
  action: "cancel_custom_edit"
  consumeEvent: true
} | {
  action: "delete_custom_reply"
  consumeEvent: true
} | {
  action: "append_custom_reply"
  input: string
  consumeEvent: true
} | {
  action: "cycle"
  delta: number
  consumeEvent: true
} | {
  action: "begin_custom_edit"
  selectedIndex: number
  consumeEvent: true
} | {
  action: "submit"
  choiceIndex?: number | undefined
  consumeEvent: true
}

export function interactionChoiceCount(interaction: RuntimeInteraction): number {
  return interaction.choices.length + (interaction.custom_choice ? 1 : 0)
}

export function interactionCustomChoiceIndex(interaction: RuntimeInteraction): number {
  return interaction.custom_choice ? interaction.choices.length : -1
}

export function resolveInteractionChoiceSubmission(options: {
  interaction: RuntimeInteraction
  requestedIndex?: number | undefined
  selectedIndex?: number | null | undefined
  customReply: string
}): InteractionChoiceSubmissionDecision {
  const interaction = options.interaction
  const maxIndex = Math.max(0, interactionChoiceCount(interaction) - 1)
  const selectedIndex = Math.min(options.requestedIndex ?? options.selectedIndex ?? 0, maxIndex)
  const customChoice = interaction.custom_choice && selectedIndex === interaction.choices.length
    ? interaction.custom_choice
    : null
  const choice = customChoice ? null : interaction.choices[selectedIndex]
  const fixedChoiceWithCustomReply = Boolean(choice && interaction.custom_choice && options.customReply)
  if (!choice) {
    if (!customChoice) {
      return { action: "unavailable" }
    }
    const minLength = customChoice.min_length ?? 1
    if (options.customReply.length < minLength) {
      return { action: "edit_custom" }
    }
  }
  return {
    action: "submit",
    selectedIndex,
    choiceId: customChoice?.id ?? choice!.id,
    customReply: customChoice || fixedChoiceWithCustomReply ? options.customReply : null,
  }
}

export function nextInteractionChoiceIndex(options: {
  interaction: RuntimeInteraction
  currentIndex: number
  delta: number
}): number | null {
  const count = interactionChoiceCount(options.interaction)
  if (count <= 0) {
    return null
  }
  return (options.currentIndex + options.delta + count) % count
}

export function appendInteractionCustomReply(options: {
  current: string
  input: string
  maxLength?: number | null | undefined
}): string {
  const maxLength = options.maxLength ?? 2000
  return options.current.length < maxLength ? `${options.current}${options.input}` : options.current
}

export function deleteInteractionCustomReply(current: string): string {
  return current.slice(0, -1)
}

export function shouldEditCustomInteractionOnEnter(options: {
  interaction: RuntimeInteraction
  selectedIndex: number
  customReply: string
}): boolean {
  return Boolean(
    options.interaction.custom_choice
      && options.selectedIndex === interactionCustomChoiceIndex(options.interaction)
      && !options.customReply,
  )
}

export function resolveInteractionChoiceKeyAction(options: {
  interaction: RuntimeInteraction
  event: InteractionChoiceKeyEvent
  selectedIndex: number
  customEditing: boolean
  customReply: string
}): InteractionChoiceKeyAction {
  const { interaction, event } = options
  if (event.eventType === "release") {
    return { action: "ignore" }
  }

  const customIndex = interactionCustomChoiceIndex(interaction)
  if (interaction.custom_choice && options.customEditing) {
    if (event.name === "escape") {
      return { action: "cancel_custom_edit", consumeEvent: true }
    }
    if (event.name === "backspace") {
      return { action: "delete_custom_reply", consumeEvent: true }
    }
    if (event.name === "return" || event.name === "enter") {
      return { action: "submit", choiceIndex: customIndex, consumeEvent: true }
    }
    if (!event.ctrl && !event.meta && !event.alt && event.name.length === 1) {
      return { action: "append_custom_reply", input: event.name, consumeEvent: true }
    }
    return { action: "handled", consumeEvent: false }
  }

  if (event.name === "left" || event.name === "up") {
    return { action: "cycle", delta: -1, consumeEvent: true }
  }
  if (event.name === "right" || event.name === "down") {
    return { action: "cycle", delta: 1, consumeEvent: true }
  }

  const numericIndex = Number.parseInt(event.name, 10)
  const choiceCount = interactionChoiceCount(interaction)
  if (Number.isInteger(numericIndex) && numericIndex >= 1 && numericIndex <= choiceCount) {
    if (interaction.custom_choice && numericIndex - 1 === customIndex) {
      return { action: "begin_custom_edit", selectedIndex: customIndex, consumeEvent: true }
    }
    return { action: "submit", choiceIndex: numericIndex - 1, consumeEvent: true }
  }

  if (event.name === "return" || event.name === "enter") {
    if (shouldEditCustomInteractionOnEnter({
      interaction,
      selectedIndex: options.selectedIndex,
      customReply: options.customReply,
    })) {
      return { action: "begin_custom_edit", selectedIndex: customIndex, consumeEvent: true }
    }
    return { action: "submit", consumeEvent: true }
  }

  return { action: "ignore" }
}
