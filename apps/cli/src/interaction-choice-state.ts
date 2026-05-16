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
    customReply: customChoice ? options.customReply : null,
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
