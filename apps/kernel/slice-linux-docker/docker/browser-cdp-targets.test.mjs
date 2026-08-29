import assert from "node:assert/strict"
import test from "node:test"

import {
  browserPageTargets,
  fallbackBrowserPageTarget,
  selectBrowserPageTarget,
} from "./browser-cdp-targets.mjs"

const page = (id, url) => ({
  id,
  type: "page",
  url,
  webSocketDebuggerUrl: `ws://127.0.0.1/${id}`,
})

test("browser page targets exclude non-page and unavailable targets", () => {
  assert.deepEqual(browserPageTargets([
    page("active", "https://example.com"),
    { id: "worker", type: "service_worker", webSocketDebuggerUrl: "ws://worker" },
    { id: "closed", type: "page", url: "about:blank" },
  ]).map((target) => target.id), ["active"])
})

test("browser target selection follows the visible tab instead of URL preferences", async () => {
  const targets = [
    page("fixture", "http://host/chariox-slice-screen-test"),
    page("visible", "https://mail.example/inbox"),
  ]

  const selected = await selectBrowserPageTarget(targets, async (target) => target.id === "visible")

  assert.equal(selected?.id, "visible")
})

test("browser target selection keeps the stable URL fallback when visibility cannot be read", async () => {
  const targets = [
    page("blank", "about:blank"),
    page("workspace", "file:///workspace/index.html"),
    page("fixture", "http://host/chariox-slice-screen-test"),
  ]

  const selected = await selectBrowserPageTarget(targets, async () => false)

  assert.equal(selected?.id, "fixture")
  assert.equal(fallbackBrowserPageTarget(targets)?.id, "fixture")
})

test("browser target selection does not probe a single page", async () => {
  let probes = 0
  const selected = await selectBrowserPageTarget([
    page("only", "https://example.com"),
  ], async () => {
    probes += 1
    return true
  })

  assert.equal(selected?.id, "only")
  assert.equal(probes, 0)
})
