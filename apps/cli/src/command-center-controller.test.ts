import assert from "node:assert/strict"
import test from "node:test"

import {
  createCommandCenterController,
  type CommandCenterKeyEvent,
  type CommandCenterRenderState,
} from "./command-center-controller.js"
import { loadCommandCenterTestCatalog } from "./command-center-test-catalog.js"
import type { CommandNode } from "./command-center-tree-projection.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"

const commandTree = loadCommandCenterTestCatalog()

test("Enter leaves a fully typed agent profile command for the normal submit handler", () => {
  for (const prompt of ["/agent provider reviewer opencode", "/agent model reviewer codex/gpt-6-luna"]) {
    const harness = createHarness(prompt, [{
      id: "profile-scope",
      label: "Profile options",
      description: "Profile options",
      value: `${prompt} `,
      children: [{ id: "child-option", label: "Child option", description: "Child option", value: `${prompt} child` }],
    }])
    harness.controller.sync(prompt)
    assert.equal(harness.controller.open(), true)
    assert.equal(harness.controller.handleKey({ name: "return" }), false)
    assert.equal(harness.controller.selectFromSubmit(), false)
    assert.equal(harness.promptText, prompt)
    assert.deepEqual(harness.executed, [])
  }
})

function createHarness(initialPrompt = "", tree: readonly CommandNode[] = commandTree, execute?: () => Promise<void>) {
  let promptText = initialPrompt
  const executed: string[] = []
  const errors: unknown[] = []
  const renderStates: CommandCenterRenderState[] = []
  const renderBoxes: Array<string | undefined> = []
  const controller = createCommandCenterController<string>({
    getCommandTree: () => tree,
    getProviderCatalog: fallbackProviderCatalog,
    getProviderCommandCatalogs: fallbackProviderCommandCatalogs,
    getCurrentProvider: () => "opencode",
    getFocusedProvider: () => "opencode",
    getCurrentModel: () => "opencode/gpt-5.4",
    getCurrentVariant: () => "high",
    getAgentAliases: () => ["reviewer", "goal-worker", "agent-1"],
    getPromptText: () => promptText,
    replacePromptText: (value) => {
      promptText = value
    },
    executeCommand: async (value) => {
      executed.push(value)
      await execute?.()
    },
    onCommandError: (error) => {
      errors.push(error)
    },
    render: (state, box) => {
      renderStates.push({
        ...state,
        items: [...state.items],
      })
      renderBoxes.push(box)
    },
  })

  return {
    controller,
    executed,
    errors,
    renderStates,
    renderBoxes,
    get promptText() {
      return promptText
    },
    set promptText(value: string) {
      promptText = value
    },
  }
}

function handledKey(name: string, overrides: Partial<CommandCenterKeyEvent> = {}) {
  let prevented = false
  let stopped = false
  return {
    event: {
      name,
      ...overrides,
      preventDefault: () => {
        prevented = true
      },
      stopPropagation: () => {
        stopped = true
      },
    },
    get prevented() {
      return prevented
    },
    get stopped() {
      return stopped
    },
  }
}

test("command center controller syncs suggestions and render state", () => {
  const harness = createHarness("/")

  harness.controller.assignBox("command-center-box")
  harness.controller.sync()

  assert.equal(harness.controller.query(), "/")
  assert.equal(harness.controller.open(), true)
  assert.equal(harness.controller.items().some((item) => item.label === "/provider"), true)
  assert.equal(harness.renderStates.at(-1)?.open, true)
  assert.equal(harness.renderBoxes.at(-1), "command-center-box")
})

test("command center controller owns keyboard selection state", () => {
  const harness = createHarness("/")
  harness.controller.sync()
  const initialIndex = harness.controller.selectedIndex()
  const down = handledKey("down")

  assert.equal(harness.controller.handleKey(down.event), true)

  assert.equal(down.prevented, true)
  assert.equal(down.stopped, true)
  assert.equal(harness.controller.selectedIndex(), initialIndex + 1)

  const up = handledKey("p", { ctrl: true })
  assert.equal(harness.controller.handleKey(up.event), true)
  assert.equal(harness.controller.selectedIndex(), initialIndex)
})

test("command center controller completes expandable items without executing", () => {
  const harness = createHarness("/workflow")
  harness.controller.sync()

  const tab = handledKey("tab")
  assert.equal(harness.controller.handleKey(tab.event), true)

  assert.equal(harness.promptText, "/workflow ")
  assert.equal(harness.executed.length, 0)
  assert.equal(harness.controller.query(), "/workflow ")
})

test("command center controller completes a filtered agent route without executing it", () => {
  const harness = createHarness("@go")
  harness.controller.sync()

  assert.equal(harness.controller.open(), true)
  assert.deepEqual(harness.controller.items().map((item) => item.label), ["@goal-worker"])

  const enter = handledKey("enter")
  assert.equal(harness.controller.handleKey(enter.event), true)

  assert.equal(harness.promptText, "@goal-worker ")
  assert.deepEqual(harness.executed, [])
  assert.equal(harness.controller.open(), false)
})

test("command center controller opens misc children without executing", () => {
  const harness = createHarness("/misc")
  harness.controller.sync()

  const enter = handledKey("enter")
  assert.equal(harness.controller.handleKey(enter.event), true)

  assert.equal(harness.promptText, "/misc ")
  assert.deepEqual(harness.executed, [])
  assert.equal(harness.controller.query(), "/misc ")
  assert.deepEqual(harness.controller.items().map((item) => item.value), [
    "/misc ",
    "/attach ",
    "/stop",
    "/waiting",
    "/exit",
  ])
})

test("command center controller executes selected commands and clears prompt", () => {
  const harness = createHarness("/exit")
  harness.controller.sync()

  const enter = handledKey("enter")
  assert.equal(harness.controller.handleKey(enter.event), true)

  assert.deepEqual(harness.executed, ["/exit"])
  assert.equal(harness.promptText, "")
  assert.equal(harness.controller.open(), false)
})

test("typed Browser access commands are not replaced by kernel access matches", () => {
  for (const prompt of ["/access list", "/access tabs", "/access revoke agent-1", "/access revoke all"]) {
    for (const entry of ["keyboard", "submit"] as const) {
      const harness = createHarness(prompt)
      harness.controller.sync()
      if (prompt === "/access list") {
        assert.ok(harness.controller.items().length > 0, "exercise a real fuzzy catalog match")
      }
      assert.equal(entry === "keyboard"
        ? harness.controller.handleKey({ name: "return" })
        : harness.controller.selectFromSubmit(), false, `${entry}: ${prompt}`)
      assert.equal(harness.promptText, prompt)
      assert.deepEqual(harness.executed, [])
    }
  }
})

test("command center controller lets exact leaf commands submit normally", () => {
  const harness = createHarness("/exit")
  harness.controller.sync()

  assert.equal(harness.controller.selectFromSubmit(), false)

  assert.deepEqual(harness.executed, [])
  assert.equal(harness.controller.open(), false)
  assert.equal(harness.controller.query(), "")
  assert.equal(harness.promptText, "/exit")
})

test("command center controller submits an exact /app install instead of completing /app", () => {
  const harness = createHarness("/app install")
  harness.controller.sync()

  assert.equal(harness.controller.selectedItem()?.value, "/app install")
  assert.equal(harness.controller.selectFromSubmit(), false)

  assert.deepEqual(harness.executed, [])
  assert.equal(harness.promptText, "/app install")
})

test("command center lets event authorization commands with arguments submit normally", () => {
  const command = "/workflow trigger event authorize dev.chariox.dummy"
  const harness = createHarness(command)
  harness.controller.sync()

  assert.deepEqual(harness.controller.items(), [])
  assert.equal(harness.controller.open(), false)
  assert.equal(harness.controller.selectFromSubmit(), false)
  assert.deepEqual(harness.executed, [])
  assert.equal(harness.promptText, command)
})

test("command center controller bypasses session alias submit selection", () => {
  const harness = createHarness("/session docs")
  harness.controller.sync()

  assert.equal(harness.controller.selectFromSubmit(), false)

  assert.deepEqual(harness.executed, [])
  assert.equal(harness.promptText, "/session docs")
})

test("command center controller submits typed arguments instead of the command they extend", () => {
  // The palette lists "/tool run " because its description mentions the typed
  // words; Enter must not run it without the arguments.
  const tree: CommandNode[] = [{
    id: "tool",
    label: "/tool",
    description: "Tools",
    value: "/tool ",
    children: [{ id: "tool-run", label: "run", description: "Run a tool, or run fast with --fast", value: "/tool run " }],
  }]
  const harness = createHarness("/tool run fast", tree)
  harness.controller.sync()
  assert.equal(harness.controller.selectedItem()?.value, "/tool run ")

  const enter = handledKey("enter")
  assert.equal(harness.controller.handleKey(enter.event), false)
  assert.equal(enter.prevented, false)
  // The prompt's own submit then sends the typed line.
  assert.equal(harness.controller.selectFromSubmit(), false)
  assert.deepEqual(harness.executed, [])
  assert.equal(harness.promptText, "/tool run fast")
})

test("command center controller runs every typed /app subcommand the TUI implements", async () => {
  // Each must be offered as itself, not completed to /app or to a sibling.
  for (const command of [
    "/app list", "/app set", "/app status", "/app journal", "/app logs", "/app worker", "/app start",
    "/app stop", "/app restart", "/app open", "/app install", "/app uninstall", "/app update", "/app dev",
    "/app dev stop",
    "/app operation", "/app cancel", "/app publisher enroll", "/app publisher status", "/app publisher cancel",
    "/app automation list", "/app automation add", "/app automation disable", "/app inbox list",
    "/app inbox add", "/app inbox remove", "/app inbox test", "/app connection list", "/app connection grant",
    "/app connection revoke", "/app file grant", "/app file save",
  ]) {
    const harness = createHarness(command)
    harness.controller.sync()
    assert.equal(harness.controller.selectedItem()?.value.trim(), command, command)
    assert.equal(harness.controller.handleKey(handledKey("enter").event), true, command)
    await Promise.resolve()
    assert.equal(harness.executed[0]?.trim(), command, command)
  }
})

test("command center controller submits /app dev stop as typed", () => {
  const harness = createHarness("/app dev stop")
  harness.controller.sync()

  assert.equal(harness.controller.selectedItem()?.value, "/app dev stop")
  assert.equal(harness.controller.selectFromSubmit(), false)
  assert.deepEqual(harness.executed, [])
  assert.equal(harness.promptText, "/app dev stop")
})

for (const rejects of [false, true]) {
  test(`MP-08 command center preserves a new draft after delayed model choice ${rejects ? "failure" : "success"}`, async () => {
    let finish!: () => void
    const pending = new Promise<void>(resolve => { finish = resolve })
    const harness = createHarness("/model ", commandTree, async () => {
      await pending
      if (rejects) throw new Error("model unavailable")
    })
    harness.controller.sync()
    assert.equal(harness.controller.selectedItem()?.kind, "model")
    assert.equal(harness.controller.handleKey(handledKey("return").event), true)
    assert.equal(harness.executed.length, 1)
    harness.promptText = "Read the public README"
    harness.controller.sync()
    finish()
    await new Promise<void>(resolve => setImmediate(resolve))
    assert.equal(harness.promptText, "Read the public README")
    assert.equal(harness.controller.query(), "Read the public README")
    assert.equal(harness.errors.length, rejects ? 1 : 0)
  })
}
