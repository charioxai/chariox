import assert from "node:assert/strict"
import test from "node:test"
import { fileURLToPath } from "node:url"

import { selkiesWebDepsContractPath } from "../apps/kernel/slice-linux-docker/docker/selkies-web-deps.contract.test.mjs"

const expectedContractPath = fileURLToPath(
  new URL(
    "../apps/kernel/slice-linux-docker/docker/selkies-web-deps.contract.test.mjs",
    import.meta.url,
  ),
)

test("root test discovery reaches the exact Selkies web dependency contract", () => {
  assert.equal(selkiesWebDepsContractPath, expectedContractPath)
})
