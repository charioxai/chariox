type TranscriptRenderDeferralControllerOptions<Renderable> = {
  isBatched: () => boolean
  getRenderable: () => Renderable | null | undefined
  requestRender: (renderable: Renderable | null | undefined) => void
}

export type TranscriptRenderDeferralController = {
  request(): void
  flush(): void
  hasPending(): boolean
}

export function createTranscriptRenderDeferralController<Renderable>(
  options: TranscriptRenderDeferralControllerOptions<Renderable>,
): TranscriptRenderDeferralController {
  let pendingRender = false

  return {
    request() {
      if (options.isBatched()) {
        pendingRender = true
        return
      }
      options.requestRender(options.getRenderable())
    },
    flush() {
      if (!pendingRender) {
        return
      }
      pendingRender = false
      options.requestRender(options.getRenderable())
    },
    hasPending() {
      return pendingRender
    },
  }
}
