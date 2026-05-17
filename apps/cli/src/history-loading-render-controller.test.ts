import assert from "node:assert/strict"
import test from "node:test"

import { createHistoryLoadingRenderController } from "./history-loading-render-controller.js"

test("history loading render controller delegates current refs and loading state", () => {
  const rendered: unknown[] = []
  let loading = true
  const controller = createHistoryLoadingRenderController({
    renderer: "renderer",
    loading: () => loading,
    renderIndicator: (options) => {
      rendered.push(options)
    },
  })

  controller.assignBox("box-a")
  controller.render()
  const first = rendered[0] as {
    box: string
    text: string | undefined
    loading: boolean
    renderer: string
    assignText: (value: string | undefined) => void
  }
  first.assignText("text-b")
  controller.assignBox("box-b")
  loading = false
  controller.render()

  assert.equal(rendered.length, 2)
  assert.equal(first.box, "box-a")
  assert.equal(first.text, undefined)
  assert.equal(first.loading, true)
  assert.equal(first.renderer, "renderer")
  assert.equal(controller.getBox(), "box-b")

  const second = rendered[1] as { box: string; text: string; loading: boolean; renderer: string }
  assert.equal(second.box, "box-b")
  assert.equal(second.text, "text-b")
  assert.equal(second.loading, false)
  assert.equal(second.renderer, "renderer")
})
