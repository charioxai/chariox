import assert from "node:assert/strict"
import test from "node:test"

import {
  createCommandCenterController,
  type CommandCenterKeyEvent,
  type CommandCenterRenderState,
} from "./command-center-controller.js"
import { loadCommandCenterTestCatalog } from "./command-center-test-catalog.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"

const commandTree = loadCommandCenterTestCatalog()

function createHarness(initialPrompt = "") {
  let promptText = initialPrompt
  const executed: string[] = []
  const errors: unknown[] = []
  const renderStates: CommandCenterRenderState[] = []
  const renderBoxes: Array<string | undefined> = []
  const controller = createCommandCenterController<string>({
    getCommandTree: () => commandTree,
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

test("command center controller lets exact leaf commands submit normally", () => {
  const harness = createHarness("/exit")
  harness.controller.sync()

  assert.equal(harness.controller.selectFromSubmit(), false)

  assert.deepEqual(harness.executed, [])
  assert.equal(harness.controller.open(), false)
  assert.equal(harness.controller.query(), "")
  assert.equal(harness.promptText, "/exit")
})

test("command center controller bypasses session alias submit selection", () => {
  const harness = createHarness("/session docs")
  harness.controller.sync()

  assert.equal(harness.controller.selectFromSubmit(), false)

  assert.deepEqual(harness.executed, [])
  assert.equal(harness.promptText, "/session docs")
})
