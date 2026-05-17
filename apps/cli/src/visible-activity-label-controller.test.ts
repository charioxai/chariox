import assert from "node:assert/strict"
import test from "node:test"

import { createVisibleActivityLabelController } from "./visible-activity-label-controller.js"

test("visible activity label controller syncs focused activity into active status", () => {
  const labels: Array<string | null> = []
  let focusedLabel: string | null = "reading"
  const controller = createVisibleActivityLabelController({
    focusedActivityLabel: () => focusedLabel,
    setActiveStatusLabel: (label) => {
      labels.push(label)
    },
  })

  controller.sync()
  focusedLabel = null
  controller.sync()

  assert.deepEqual(labels, ["reading", null])
})
