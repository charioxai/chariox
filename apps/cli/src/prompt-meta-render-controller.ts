import type {
  PromptMetaPart,
  PromptUsageMeta,
} from "./prompt-meta.js"
import type { PromptMetaRenderableRefs } from "./prompt-meta-renderer.js"

export type PromptMetaRenderableRefKey = keyof PromptMetaRenderableRefs

export type PromptMetaRenderControllerDeps = {
  refs?: () => PromptMetaRenderableRefs
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
  const refs: PromptMetaRenderableRefs = {
    providerText: undefined,
    providerDividerText: undefined,
    modelText: undefined,
    modelDividerText: undefined,
    variantText: undefined,
    usageDividerText: undefined,
    usageTokensText: undefined,
    usageBarOpenText: undefined,
    usageBarFilledText: undefined,
    usageBarEmptyText: undefined,
    usageBarCloseText: undefined,
    usagePercentText: undefined,
  }

  const currentRefs = deps.refs ?? (() => refs)

  return {
    assignRef(key: PromptMetaRenderableRefKey, value: PromptMetaRenderableRefs[PromptMetaRenderableRefKey]) {
      refs[key] = value
    },
    setRenderables(parts: PromptMetaPart[]) {
      deps.renderMeta(currentRefs(), parts, deps.getUsage())
    },
  }
}
