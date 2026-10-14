import assert from "node:assert/strict"
import test from "node:test"
import { TextRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { renderPromptMeta, type PromptMetaRenderableRefs } from "./prompt-meta-renderer.js"

test("MP-08 / MP-10 late prompt metadata refresh skips destroyed refs and updates the mounted usage", async () => {
  const harness = await createTestRenderer({ width: 80, height: 24, useThread: false })
  const oldProvider = new TextRenderable(harness.renderer, { content: "Codex" })
  const mountedUsage = new TextRenderable(harness.renderer, { content: "old usage" })
  harness.renderer.root.add(oldProvider)
  harness.renderer.root.add(mountedUsage)
  try {
    oldProvider.destroy()
    const refs = { providerText: oldProvider, usageTokensText: mountedUsage } as PromptMetaRenderableRefs
    assert.doesNotThrow(() => renderPromptMeta(refs, [], { tokensLabel: "42 tok", usagePercent: 4, usageLabel: "4%", barFilled: "=", barEmpty: "---------" }))
    await harness.renderOnce()
    assert.match(harness.captureCharFrame(), /42 tok/)
  } finally { harness.renderer.destroy() }
})
