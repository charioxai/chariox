import type { RuntimeInteraction } from "./kernel-types.js"

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
  /** The text the terminal typed. Unlike `name`, it keeps letter case. */
  sequence?: string
  eventType?: string
  ctrl?: boolean
  meta?: boolean
  alt?: boolean
  shift?: boolean
}

const CONTROL_CHARACTERS = /[\u0000-\u001f\u007f-\u009f]/

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
    if (Array.from(options.customReply).length < minLength) {
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

/** Appends typed or pasted text, up to `maxLength` characters (code points,
 * as the kernel counts them). */
export function appendInteractionCustomReply(options: {
  current: string
  input: string
  maxLength?: number | null | undefined
}): string {
  const maxLength = options.maxLength ?? 2000
  const characters = Array.from(options.current)
  for (const character of options.input) {
    if (characters.length >= maxLength) break
    characters.push(character)
  }
  return characters.join("")
}

/** Removes the last character, never half of a surrogate pair. */
export function deleteInteractionCustomReply(current: string): string {
  return Array.from(current).slice(0, -1).join("")
}

/** The text a key press types into a custom reply, exactly as typed.
 *
 * A key's `name` is the lower-case key name ("a" for both a and A), so a
 * custom reply, which may be a vault passphrase, must use the typed
 * `sequence`: it keeps letter case, shifted symbols and Unicode. A key whose
 * sequence is an escape sequence (the kitty protocol without associated text)
 * falls back to its single-character name, upper-cased with Shift. Keys that
 * type nothing, and Ctrl/Alt/Meta chords, give "". */
export function interactionCustomReplyKeyText(event: InteractionChoiceKeyEvent): string {
  if (event.ctrl || event.meta || event.alt) {
    return ""
  }
  if (event.sequence && !CONTROL_CHARACTERS.test(event.sequence)) {
    return event.sequence
  }
  if (event.name === "space") {
    return " "
  }
  if (Array.from(event.name).length !== 1 || CONTROL_CHARACTERS.test(event.name)) {
    return ""
  }
  return event.shift ? event.name.toLocaleUpperCase() : event.name
}

/** The text a paste adds to a custom reply: the pasted text without its
 * trailing line break, or null when it holds a line break or another control
 * character (a custom reply is one line, and Enter submits it). */
export function interactionCustomReplyPasteText(text: string): string | null {
  const line = text.replace(/(?:\r\n|\r|\n)+$/, "")
  return CONTROL_CHARACTERS.test(line) ? null : line
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
    const input = interactionCustomReplyKeyText(event)
    if (input) {
      return { action: "append_custom_reply", input, consumeEvent: true }
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
