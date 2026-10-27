export type RawPasteKeyInput = {
  processPaste(data: string): void
  on(event: "paste", listener: (event: RawPasteEventLike) => void): unknown
  off(event: "paste", listener: (event: RawPasteEventLike) => void): unknown
}

type RawPasteEventLike = {
  text: string
  preventDefault(): void
  stopPropagation(): void
}

export type RawPasteEvent = RawPasteEventLike & {
  /** The paste as the terminal sent it, before OpenTUI strips ANSI codes. */
  rawText: string | null
}

/** Sends terminal pastes to `handlePaste` with their raw text.
 *
 * OpenTUI removes ANSI escape sequences from a bracketed paste before it
 * emits the paste event, so the event alone cannot show that the pasted text
 * held control characters. A secret must refuse such a paste, not keep a
 * silently changed value, so this keeps the payload as the terminal sent it
 * while that paste is dispatched. Returns the cleanup. */
export function routeRawPastes(
  keyInput: RawPasteKeyInput,
  handlePaste: (event: RawPasteEvent) => unknown,
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
  const listener = (event: RawPasteEventLike) => {
    handlePaste({
      text: event.text,
      rawText,
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
