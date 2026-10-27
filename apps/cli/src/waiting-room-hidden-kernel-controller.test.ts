import assert from "node:assert/strict"
import test from "node:test"

import { createWaitingRoomHiddenKernelController } from "./waiting-room-hidden-kernel-controller.js"

test("waiting room hidden kernel controller tracks and persists hidden kernels", () => {
  const persisted: string[][] = []
  const controller = createWaitingRoomHiddenKernelController({
    initialHiddenKernelIds: ["kernel-b"],
    persistHiddenKernelIds: (kernelIds) => {
      persisted.push(kernelIds)
    },
  })

  assert.equal(controller.isKernelHidden("kernel-b"), true)
  assert.equal(controller.isKernelHidden("kernel-a"), false)

  controller.hideKernel("kernel-a")

  assert.equal(controller.isKernelHidden("kernel-a"), true)
  assert.deepEqual(persisted, [["kernel-a", "kernel-b"]])
})
