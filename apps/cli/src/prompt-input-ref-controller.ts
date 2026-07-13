export type PromptInputRefRenderable = {
  height: number
  focused: boolean
  plainText: string
  syntaxStyle: unknown
  isDestroyed?: boolean
  focus: () => void
  blur: () => void
  clear: () => void
}

export function createPromptInputRefController<TInput extends PromptInputRefRenderable>() {
  let input: TInput | undefined

  function liveInput(): TInput | undefined {
    if (input?.isDestroyed) {
      input = undefined
    }
    return input
  }

  return {
    assignInput(value: TInput | undefined) {
      input = value
    },
    current() {
      return liveInput()
    },
    currentOrNull() {
      return liveInput() ?? null
    },
    hasInput() {
      return Boolean(liveInput())
    },
    height(fallback: number) {
      return liveInput()?.height ?? fallback
    },
    isFocused() {
      return Boolean(liveInput()?.focused)
    },
    plainText() {
      return liveInput()?.plainText
    },
    setSyntaxStyle(style: unknown) {
      const current = liveInput()
      if (current) {
        current.syntaxStyle = style
      }
    },
    focus() {
      liveInput()?.focus()
    },
    blur() {
      liveInput()?.blur()
    },
    clear() {
      liveInput()?.clear()
    },
  }
}
