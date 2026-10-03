import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { readFile, readdir } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import test from "node:test"

import {
  isReviewedManagedParityModuleVerification,
  loadReviewedManagedParityAdapterModule,
} from "./managed-browser-computer-parity-cli.mjs"

test("real product transport loads through the reviewed adapter entry without running a product action", async () => {
  const url = new URL("./managed-browser-computer-parity-product-transport.mjs", import.meta.url)
  const modulePath = fileURLToPath(url)
  const expected = {
    identity: "chariox-managed-parity-product-transport-v1",
    sha256: `sha256:${createHash("sha256").update(await readFile(url)).digest("hex")}`,
  }
  const { imported, verification } = await loadReviewedManagedParityAdapterModule(modulePath, expected)
  assert.equal(imported.MANAGED_BROWSER_COMPUTER_PARITY_ADAPTER_IDENTITY, expected.identity)
  assert.equal(typeof imported.createManagedBrowserComputerParityTransport, "function")
  assert.equal(isReviewedManagedParityModuleVerification(verification, "adapter"), true)
  assert.equal(isReviewedManagedParityModuleVerification(verification, "inspector"), false)
  await assert.rejects(loadReviewedManagedParityAdapterModule(modulePath, {
    ...expected, identity: "unreviewed-transport",
  }), /identity does not match reviewed pin/)
  await assert.rejects(loadReviewedManagedParityAdapterModule(modulePath, {
    ...expected, sha256: `sha256:${"0".repeat(64)}`,
  }), /hash does not match reviewed pin/)
  const remaining = await readdir(new URL(".", url))
  assert.equal(remaining.some(name => name.startsWith(".managed-browser-computer-parity-product-transport.mjs.")), false)
})
