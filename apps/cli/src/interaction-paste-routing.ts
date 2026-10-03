import type { FocusedInteractionChoicePasteEvent } from "./focused-interaction-choice-controller.js"

export type PasteKeyInput = {
  processPaste(data: string): void
  on(event: "paste", listener: (event: PasteEventLike) => void): unknown
  off(event: "paste", listener: (event: PasteEventLike) => void): unknown
}

type PasteEventLike = {
  text: string
  defaultPrevented: boolean
  preventDefault(): void
  stopPropagation(): void
}

/** Sends terminal pastes to the focused interaction's custom reply.
 *
 * OpenTUI removes ANSI escape sequences from a bracketed paste before it
 * emits the paste event, so the event alone cannot show that the pasted text
 * held control characters. A secret reply must refuse such a paste, not keep
 * a silently changed value, so this keeps the payload as the terminal sent it
 * while that paste is dispatched. Returns the cleanup. */
export function routeInteractionPastes(
  keyInput: PasteKeyInput,
  handlePaste: (event: FocusedInteractionChoicePasteEvent) => boolean,
  blocked: () => boolean,
): () => void {
  const processPaste = keyInput.processPaste
  let rawText: string | null = null
  keyInput.processPaste = function (this: unknown, data: string) {
    rawText = data
    try {
      processPaste.call(this, data)
    } finally {
      rawText = null
    }
  }
  const listener = (event: PasteEventLike) => {
    if (blocked()) return
    handlePaste({
      text: event.text,
      rawText,
      defaultPrevented: event.defaultPrevented,
      preventDefault: () => event.preventDefault(),
      stopPropagation: () => event.stopPropagation(),
    })
  }
  keyInput.on("paste", listener)
  return () => {
    keyInput.off("paste", listener)
    keyInput.processPaste = processPaste
  }
}
