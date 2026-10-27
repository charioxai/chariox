export type ResponsePaneRenderScheduleControllerDeps<TRenderable> = {
  responseLayoutBox: () => TRenderable | undefined
  historyLoadingBox: () => TRenderable | undefined
  requestTree: (renderable: TRenderable | undefined) => void
  requestRoot: () => void
}

export function createResponsePaneRenderScheduleController<TRenderable>(
  deps: ResponsePaneRenderScheduleControllerDeps<TRenderable>,
) {
  return {
    scheduleRepaint() {
      deps.requestTree(deps.responseLayoutBox())
      deps.requestTree(deps.historyLoadingBox())
      deps.requestRoot()
    },
  }
}
