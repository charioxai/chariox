import type {
  PromptMetaPart,
  PromptUsageMeta,
} from "./prompt-meta.js"
import type { PromptMetaRenderableRefs } from "./prompt-meta-renderer.js"

export type PromptMetaRenderControllerDeps = {
  refs: () => PromptMetaRenderableRefs
  getUsage: () => PromptUsageMeta | null
  renderMeta: (
    refs: PromptMetaRenderableRefs,
    parts: PromptMetaPart[],
    usage: PromptUsageMeta | null,
  ) => void
}

export function createPromptMetaRenderController(
  deps: PromptMetaRenderControllerDeps,
) {
  return {
    setRenderables(parts: PromptMetaPart[]) {
      deps.renderMeta(deps.refs(), parts, deps.getUsage())
    },
  }
}
