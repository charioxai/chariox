type HistoryLoadingRenderOptions<TRenderer, TBox, TText> = {
  box: TBox | undefined
  text: TText | undefined
  loading: boolean
  renderer: TRenderer
  assignText: (text: TText | undefined) => void
}

export type HistoryLoadingRenderControllerDeps<TRenderer, TBox, TText> = {
  renderer: TRenderer
  box: () => TBox | undefined
  text: () => TText | undefined
  loading: () => boolean
  assignText: (text: TText | undefined) => void
  renderIndicator: (options: HistoryLoadingRenderOptions<TRenderer, TBox, TText>) => void
}

export function createHistoryLoadingRenderController<TRenderer, TBox, TText>(
  deps: HistoryLoadingRenderControllerDeps<TRenderer, TBox, TText>,
) {
  return {
    render() {
      deps.renderIndicator({
        box: deps.box(),
        text: deps.text(),
        loading: deps.loading(),
        renderer: deps.renderer,
        assignText: deps.assignText,
      })
    },
  }
}
