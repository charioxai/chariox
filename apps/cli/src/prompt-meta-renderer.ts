import {
  TextAttributes,
  type TextRenderable,
} from "@opentui/core"

import type {
  PromptMetaPart,
  PromptUsageMeta,
} from "@arroba/kernel-client/prompt-meta"
import { theme } from "./theme.js"

export type PromptMetaRenderableRefs = {
  providerText: TextRenderable | undefined
  providerDividerText: TextRenderable | undefined
  modelText: TextRenderable | undefined
  modelDividerText: TextRenderable | undefined
  variantText: TextRenderable | undefined
  usageDividerText: TextRenderable | undefined
  usageTokensText: TextRenderable | undefined
  usageBarOpenText: TextRenderable | undefined
  usageBarFilledText: TextRenderable | undefined
  usageBarEmptyText: TextRenderable | undefined
  usageBarCloseText: TextRenderable | undefined
  usagePercentText: TextRenderable | undefined
}

export function renderPromptMeta(
  refs: PromptMetaRenderableRefs,
  parts: PromptMetaPart[],
  usage: PromptUsageMeta | null,
): void {
  if (parts.length === 0) {
    clearPromptMetaParts(refs)
    renderUsageMeta(refs, usage)
    return
  }

  clearPromptMetaParts(refs)
  renderUsageMeta(refs, usage)
}

function clearPromptMetaParts(refs: PromptMetaRenderableRefs): void {
  setTextRenderable(refs.providerText, "", theme.textMuted)
  setTextRenderable(refs.providerDividerText, "", theme.textMuted)
  setTextRenderable(refs.modelText, "", theme.textMuted)
  setTextRenderable(refs.modelDividerText, "", theme.textMuted)
  setTextRenderable(refs.variantText, "", theme.textMuted)
}

function renderUsageMeta(refs: PromptMetaRenderableRefs, usage: PromptUsageMeta | null): void {
  setTextRenderable(refs.usageDividerText, "", theme.textMuted)
  setTextRenderable(
    refs.usageTokensText,
    usage?.tokensLabel ?? "",
    usage ? theme.secondary : theme.textMuted,
    usage ? TextAttributes.BOLD : TextAttributes.NONE,
  )
  setTextRenderable(refs.usageBarOpenText, usage?.usageLabel ? " [" : "", theme.textMuted)
  setTextRenderable(
    refs.usageBarFilledText,
    usage?.barFilled ?? "",
    theme.primary,
    usage?.barFilled ? TextAttributes.BOLD : TextAttributes.NONE,
  )
  setTextRenderable(refs.usageBarEmptyText, usage?.usageLabel ? (usage?.barEmpty ?? "") : "", theme.textMuted)
  setTextRenderable(refs.usageBarCloseText, usage?.usageLabel ? "]" : "", theme.textMuted)
  setTextRenderable(
    refs.usagePercentText,
    usage?.usageLabel ? ` ${usage.usageLabel}` : "",
    usage?.usageLabel ? theme.info : theme.textMuted,
    usage?.usageLabel ? TextAttributes.BOLD : TextAttributes.NONE,
  )
}

function setTextRenderable(
  text: TextRenderable | undefined,
  content: string,
  fg: (typeof theme)[keyof typeof theme],
  attributes = TextAttributes.NONE,
): void {
  if (!text) {
    return
  }
  text.content = content
  text.fg = fg
  text.attributes = attributes
}
