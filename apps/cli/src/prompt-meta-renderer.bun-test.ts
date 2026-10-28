// MP-08 / MP-10 / MP-11: actual destroyed OpenTUI refs during waiting-room relayout.
import assert from "node:assert/strict"
import test from "node:test"
import { TextRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { renderPromptMeta, type PromptMetaRenderableRefs } from "./prompt-meta-renderer.js"

test("review0034 prompt metadata tolerates replaced destroyed refs and renders the new mount", async () => {
  const harness = await createTestRenderer({ width: 80, height: 24, useThread: false })
  try {
    const prior = new TextRenderable(harness.renderer, { content: "previous provider" })
    prior.destroyRecursively()
    const refs: PromptMetaRenderableRefs = {providerText:prior,providerDividerText:undefined,modelText:undefined,modelDividerText:undefined,variantText:undefined,usageDividerText:undefined,usageTokensText:undefined,usageBarOpenText:undefined,usageBarFilledText:undefined,usageBarEmptyText:undefined,usageBarCloseText:undefined,usagePercentText:undefined}
    assert.doesNotThrow(() => renderPromptMeta(refs, [], null))
    const current = new TextRenderable(harness.renderer, {content:""})
    harness.renderer.root.add(current)
    refs.providerText=undefined;refs.usageTokensText=current
    renderPromptMeta(refs, [], {usagePercent:null,tokensLabel:"128 tokens",barFilled:"",barEmpty:"",usageLabel:""})
    await harness.renderOnce()
    assert(harness.captureCharFrame().includes("128 tokens"))
  } finally {
    if (!harness.renderer.isDestroyed) harness.renderer.destroy()
  }
})
