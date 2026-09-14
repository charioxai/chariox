import assert from "node:assert/strict"
import { mkdtemp, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

const productTransportModule = new URL("./managed-browser-computer-parity-product-transport.mjs", import.meta.url)

test("managed parity product transport exposes the reviewed public factory", async () => {
  const evidenceRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-managed-parity-contract-"))
  try {
    const imported = await import(productTransportModule.href)
    assert.equal(typeof imported.createManagedBrowserComputerParityTransport, "function")

    const transport = await imported.createManagedBrowserComputerParityTransport({ evidenceRoot })
    assert.equal(typeof transport?.run, "function")
  } finally {
    await rm(evidenceRoot, { recursive: true, force: true })
  }
})
