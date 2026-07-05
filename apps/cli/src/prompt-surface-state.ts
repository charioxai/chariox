export type PromptInputPlaceholderTarget = {
  placeholder: unknown
}

export function createPromptPlaceholderSyncController(deps: {
  getPromptInput: () => PromptInputPlaceholderTarget | null
  getPlaceholder: () => string
}) {
  return {
    sync() {
      const promptInput = deps.getPromptInput()
      if (!promptInput) {
        return
      }
      promptInput.placeholder = deps.getPlaceholder()
    },
  }
}
