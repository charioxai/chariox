import test from "node:test"
import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
const assets = join(dirname(fileURLToPath(import.meta.url)), "agent-trigger-assets")
test("MP-08/MP-10: the trigger page bundle ships and points at the lazy math and code renderers", async () => {
  const viewer = await readFile(join(assets, "viewer.js"), "utf8")
  assert.ok(viewer.includes("./math-renderer.js") && viewer.includes("./code-highlighter.js"), "the viewer must know where its renderers are served")
  assert.match(await readFile(join(assets, "math-renderer.js"), "utf8"), /renderMath/)
  assert.match(await readFile(join(assets, "code-highlighter.js"), "utf8"), /highlight/i)
})
