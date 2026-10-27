type HistoryLoadingRenderOptions<TRenderer, TBox, TText> = {
  box: TBox | undefined
  text: TText | undefined
  loading: boolean
  message: string | null
  renderer: TRenderer
  assignText: (text: TText | undefined) => void
}

export type HistoryLoadingRenderControllerDeps<TRenderer, TBox, TText> = {
  renderer: TRenderer
  loading: () => boolean
  message?: () => string | null
  renderIndicator: (options: HistoryLoadingRenderOptions<TRenderer, TBox, TText>) => void
}

export function createHistoryLoadingRenderController<TRenderer, TBox, TText>(
  deps: HistoryLoadingRenderControllerDeps<TRenderer, TBox, TText>,
) {
  let box: TBox | undefined
  let text: TText | undefined

  return {
    assignBox(value: TBox | undefined) {
      box = value
    },
    getBox() {
      return box
    },
    render() {
      deps.renderIndicator({
        box,
        text,
        loading: deps.loading(),
        message: deps.message?.() ?? null,
        renderer: deps.renderer,
        assignText: (value) => {
          text = value
        },
      })
    },
  }
}
