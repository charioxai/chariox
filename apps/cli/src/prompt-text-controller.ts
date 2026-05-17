type PromptTextInput = {
  plainText: string
  cursorOffset: number
  setText: (value: string) => void
  clear: () => void
}

type PromptTextControllerOptions = {
  initialText: string
  getPromptInput: () => PromptTextInput | null
  refreshHighlights: () => void
}

export type PromptTextController = {
  currentText(): string
  snapshot(): string
  setSnapshot(text: string): void
  isProgrammaticMutation(): boolean
  syncSnapshot(): string
  setText(text: string): void
  clear(): void
  cursorOffset(): number
}

export function createPromptTextController(
  options: PromptTextControllerOptions,
): PromptTextController {
  let snapshot = options.initialText
  let muting = false

  const withMutedMutation = (mutate: () => void) => {
    muting = true
    try {
      mutate()
    } finally {
      muting = false
    }
  }

  const setText = (text: string) => {
    const promptInput = options.getPromptInput()
    if (!promptInput) {
      snapshot = text
      return
    }

    withMutedMutation(() => {
      promptInput.setText(text)
      promptInput.cursorOffset = text.length
      snapshot = text
      options.refreshHighlights()
    })
  }

  return {
    currentText() {
      return options.getPromptInput()?.plainText ?? snapshot
    },
    snapshot() {
      return snapshot
    },
    setSnapshot(text) {
      snapshot = text
    },
    isProgrammaticMutation() {
      return muting
    },
    syncSnapshot() {
      snapshot = options.getPromptInput()?.plainText ?? ""
      return snapshot
    },
    setText,
    clear() {
      const promptInput = options.getPromptInput()
      if (!promptInput) {
        setText("")
        return
      }

      withMutedMutation(() => {
        promptInput.clear()
        promptInput.cursorOffset = 0
        snapshot = ""
      })
    },
    cursorOffset() {
      return options.getPromptInput()?.cursorOffset ?? snapshot.length
    },
  }
}
