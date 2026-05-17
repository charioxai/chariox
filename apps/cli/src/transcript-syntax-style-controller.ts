export type TranscriptSyntaxStyleControllerOptions<TStyle> = {
  createStyle: () => TStyle
}

export function createTranscriptSyntaxStyleController<TStyle>(
  options: TranscriptSyntaxStyleControllerOptions<TStyle>,
) {
  let style = options.createStyle()

  return {
    current: () => style,
    reset: () => {
      style = options.createStyle()
      return style
    },
  }
}
