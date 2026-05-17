export type PromptInputRefRenderable = {
  height: number
  focused: boolean
  plainText: string
  syntaxStyle: unknown
  focus: () => void
  blur: () => void
  clear: () => void
}

export function createPromptInputRefController<TInput extends PromptInputRefRenderable>() {
  let input: TInput | undefined

  return {
    assignInput(value: TInput | undefined) {
      input = value
    },
    current() {
      return input
    },
    currentOrNull() {
      return input ?? null
    },
    hasInput() {
      return Boolean(input)
    },
    height(fallback: number) {
      return input?.height ?? fallback
    },
    isFocused() {
      return Boolean(input?.focused)
    },
    plainText() {
      return input?.plainText
    },
    setSyntaxStyle(style: unknown) {
      if (input) {
        input.syntaxStyle = style
      }
    },
    focus() {
      input?.focus()
    },
    blur() {
      input?.blur()
    },
    clear() {
      input?.clear()
    },
  }
}
