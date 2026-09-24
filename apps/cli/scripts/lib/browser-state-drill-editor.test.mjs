import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import { test } from "node:test"

import { completeBrowserStateEditorHandoff, createBrowserStateEditorDrill } from "./browser-state-drill-editor.mjs"

test("graphical editor fixtures are written by the slice user in rootless Docker", async () => {
  const calls = []
  const dockerText = async (args, options = {}) => {
    calls.push({ args, options })
    if (args.includes("dpkg-query")) return "0.5.10-2\n"
    if (args.includes("sha256sum")) return "fixture hashes\n"
    if (args.some((part) => part.includes("CHARIOX_SLICE_DISPLAY"))) return ":99"
    return ""
  }
  const editor = createBrowserStateEditorDrill({
    containerName: "slice-test", runId: "test", dockerText,
  })

  await editor.install()

  for (const [name, target] of [
    ["launch.sh", "/home/slice/.config/chariox-browser-state-editor/launch.sh"],
    ["menu.xml", "/home/slice/.config/openbox/menu.xml"],
  ]) {
    const write = calls.find(({ args }) => args.some((part) => part.includes(target)))
    assert.ok(write, `missing ${name} fixture write`)
    assert.deepEqual(write.args.slice(0, 5), ["exec", "-i", "-u", "slice", "slice-test"])
    assert.deepEqual(write.options.stdin,
      await readFile(new URL(`../fixtures/browser-state-editor/${name}`, import.meta.url)))
  }
  assert.equal(calls.some(({ args }) => args[0] === "cp"), false)
})

function handoffDependencies(events, { visualMatches = [{
  text: "Fixture interactions", center_x: 640, center_y: 400,
}] } = {}) {
  let restored = false
  let windowQueryCount = 0
  return {
    finishDesktopWork: async () => { events.push("editor-closed") },
    browserWindowIds: async () => { events.push(`browser-windows-${++windowQueryCount}`); return ["42"] },
    visibleBrowserWindowIds: async () => { events.push("visible-browser-windows"); return restored ? ["42"] : [] },
    activeWindowId: async () => { events.push("active-window"); return restored ? "42" : "7" },
    taskbarBounds: async () => { events.push("taskbar-bounds"); return { x: 0, y: 766, width: 1280, height: 34 } },
    pointerClick: async (x, y) => { events.push(`pointer-click:${x},${y}`); restored = true },
    prepareBrowser: async () => { events.push("navigate-browser") },
    waitFor: async (predicate) => {
      events.push("wait-visible")
      assert.equal(await predicate(), true)
      return true
    },
    screenshot: async () => {
      events.push("screenshot")
      return { inside: "/tmp/handoff.png", path: "/evidence/browser-handoff.png" }
    },
    findText: async (query, image) => {
      events.push("screenshot-ocr")
      assert.equal(query, "Fixture interactions")
      assert.equal(image, "/tmp/handoff.png")
      return visualMatches
    },
    browserWindowBounds: async (windowId) => {
      events.push("browser-bounds")
      assert.equal(windowId, "42")
      return { x: 0, y: 40, width: 1280, height: 726 }
    },
    expectedVisibleText: "Fixture interactions",
  }
}

test("Browser handoff restores the same window after editor work and captures OCR inside it", async () => {
  const events = []
  const result = await completeBrowserStateEditorHandoff(handoffDependencies(events))

  assert.equal(result.windowId, "42")
  assert.equal(result.activation, "taskbar-pointer")
  assert.equal(result.visibleText, "Fixture interactions")
  assert.equal(result.screenshot, "/evidence/browser-handoff.png")
  const at = (event) => events.findIndex((entry) => entry === event)
  assert.ok(at("browser-windows-1") < at("editor-closed"))
  assert.ok(at("editor-closed") < at("browser-windows-2"))
  assert.ok(at("editor-closed") < at("pointer-click:126,783"))
  assert.ok(at("pointer-click:126,783") < at("navigate-browser"))
  assert.ok(at("navigate-browser") < at("screenshot"))
  assert.ok(at("screenshot") < at("screenshot-ocr"))
  assert.ok(at("screenshot-ocr") < at("browser-bounds"))
})

test("Browser handoff keeps the primary window when a transient Chromium popup closes", async () => {
  const events = []
  const dependencies = handoffDependencies(events)
  let windowQueryCount = 0
  dependencies.browserWindowIds = async () => {
    events.push(`browser-windows-${++windowQueryCount}`)
    return windowQueryCount === 1 ? ["42", "99"] : ["42"]
  }
  dependencies.browserWindowBounds = async (windowId) => {
    events.push(`browser-bounds-${windowId}`)
    return windowId === "42"
      ? { x: 0, y: 40, width: 1280, height: 726 }
      : { x: 650, y: 96, width: 320, height: 420 }
  }

  const result = await completeBrowserStateEditorHandoff(dependencies)
  assert.equal(result.windowId, "42")
  assert.ok(events.includes("browser-bounds-99"))
  assert.ok(events.includes("screenshot-ocr"))
})

test("Browser handoff refuses two equally sized browser windows", async () => {
  const events = []
  const dependencies = handoffDependencies(events)
  dependencies.browserWindowIds = async () => ["42", "99"]
  dependencies.browserWindowBounds = async () => ({ x: 0, y: 40, width: 1280, height: 726 })
  await assert.rejects(
    completeBrowserStateEditorHandoff(dependencies),
    /cannot distinguish multiple full-size Chromium windows/,
  )
  assert.ok(!events.includes("editor-closed"))
})

test("Browser handoff rejects a screenshot with no visible fixture text", async () => {
  const events = []
  await assert.rejects(
    completeBrowserStateEditorHandoff(handoffDependencies(events, { visualMatches: [] })),
    /screenshot OCR did not find visible Browser content/,
  )
  assert.ok(events.indexOf("editor-closed") < events.indexOf("pointer-click:126,783"))
  assert.ok(events.includes("screenshot"))
  assert.ok(events.includes("screenshot-ocr"))
  assert.ok(!events.includes("browser-bounds"))
})

test("Browser handoff rejects OCR that appears outside the Chromium window", async () => {
  const events = []
  await assert.rejects(
    completeBrowserStateEditorHandoff(handoffDependencies(events, { visualMatches: [{
      text: "Fixture interactions", center_x: 640, center_y: 783,
    }] })),
    /OCR text was not inside the same Chromium window/,
  )
})

test("Browser handoff rejects replacing the Chromium window during preparation", async () => {
  const events = []
  const dependencies = handoffDependencies(events)
  let windowQueryCount = 0
  dependencies.browserWindowIds = async () => {
    windowQueryCount++
    events.push(`browser-windows-${windowQueryCount}`)
    return windowQueryCount < 3 ? ["42"] : ["43"]
  }
  await assert.rejects(
    completeBrowserStateEditorHandoff(dependencies),
    /replaced the Chromium window selected at handoff/,
  )
  assert.ok(events.includes("navigate-browser"))
  assert.ok(!events.includes("screenshot"))
})
