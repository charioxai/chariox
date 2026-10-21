import assert from "node:assert/strict"
import test from "node:test"
import { createProjectEnvironmentReadController } from "./project-environment-read-controller.js"

import {
  createCliStdinKeyController,
  type CliStdinKeyControllerDeps,
  type CliStdinKeyEvent,
} from "./cli-stdin-key-controller.js"

test("cli stdin key controller ignores unparsable input", () => {
  const harness = createHarness({ parsedEvent: null })

  assert.equal(harness.controller.handleData("x"), false)
  assert.deepEqual(harness.calls(), ["parse:x"])
})

test("MP-08 / MP-10 key presses clear selection; key releases preserve it", () => {
  for (const [event, shouldClear] of [
    [keyEvent("x", { eventType: "press" }), true],
    [keyEvent("x", { eventType: "release" }), false],
  ] as const) {
    let clears = 0
    const harness = createHarness({ parsedEvent: event, clearTextSelection: () => { clears++ } })
    harness.controller.handleData("x")
    assert.equal(clears, shouldClear ? 1 : 0)
  }
})

test("cli stdin key controller closes dialog overlays on escape before other handlers", () => {
  const harness = createHarness({
    dialogOpen: true,
    sessionBrowserHandled: true,
    parsedEvent: keyEvent("escape"),
  })

  assert.equal(harness.controller.handleData("x"), true)
  assert.deepEqual(harness.calls(), ["parse:x", "close-dialog"])
})

test("cli stdin key controller delegates session browser keys early", () => {
  const harness = createHarness({
    sessionBrowserHandled: true,
    parsedEvent: keyEvent("down"),
  })

  assert.equal(harness.controller.handleData("x"), true)
  assert.deepEqual(harness.calls(), ["parse:x", "session-browser:down"])
})

test("cli stdin key controller routes exit before prompt state", () => {
  const exitHarness = createHarness({ parsedEvent: keyEvent("e", { ctrl: true }) })
  assert.equal(exitHarness.controller.handleData("x"), true)
  assert.deepEqual(exitHarness.calls(), ["parse:x", "session-browser:e", "exit"])
})

test("cli stdin key controller yields focused interactions to the focused prompt input", () => {
  const promptHarness = createHarness({
    focusedInteractionActive: true,
    focusedInteractionHandled: true,
    promptFocused: true,
    parsedEvent: keyEvent("p"),
  })
  assert.equal(promptHarness.controller.handleData("x"), true)
  assert.deepEqual(promptHarness.calls(), [
    "parse:x",
    "session-browser:p",
  ])

  const focusedHarness = createHarness({
    focusedInteractionHandled: true,
    parsedEvent: keyEvent("return"),
  })
  assert.equal(focusedHarness.controller.handleData("x"), true)
  assert.deepEqual(focusedHarness.calls(), ["parse:x", "session-browser:return", "focused:return"])
})

test("cli stdin key controller delegates queued prompt shortcuts before command center prompt input", () => {
  const queuedEvents: CliStdinKeyEvent[] = []
  const harness = createHarness({
    queuedPromptHandled: true,
    promptFocused: true,
    commandCenterOpen: true,
    parsedEvent: keyEvent("s", { meta: true }),
    onQueuedPromptKey: (event) => {
      queuedEvents.push(event)
      return true
    },
  })

  assert.equal(harness.controller.handleData("x"), true)
  const queuedEvent = queuedEvents[0]
  assert.ok(queuedEvent)
  assert.equal(queuedEvent?.alt, true)
  assert.equal(queuedEvent?.meta, false)
  assert.deepEqual(harness.calls(), [
    "parse:x",
    "session-browser:s",
    "focused:s",
    "queued-prompt:s",
  ])
})

test("MP-08 / MP-10 queued-prompt Alt keys reach the strip while a focused interaction owns the prompt", () => {
  for (const name of ["s", "c", "j", "k"]) {
    const steered: CliStdinKeyEvent[] = []
    const harness = createHarness({
      promptFocused: true,
      focusedInteractionActive: true,
      parsedEvent: keyEvent(name, { meta: true }),
      onQueuedPromptKey: (event) => { steered.push(event); return true },
    })

    assert.equal(harness.controller.handleData("x"), true, name)
    assert.equal(steered.length, 1, name)
    assert.equal(steered[0]?.alt, true, name)
  }
})

test("MP-08 / MP-10 a focused interaction still owns every other key, including Alt+Up and Alt+Down", () => {
  for (const parsedEvent of [keyEvent("x"), keyEvent("s"), keyEvent("up", { meta: true }), keyEvent("down", { meta: true }), keyEvent("s", { meta: true, shift: true })]) {
    const harness = createHarness({ promptFocused: true, focusedInteractionActive: true, parsedEvent })
    assert.equal(harness.controller.handleData("x"), true)
    assert.ok(!harness.calls().some(call => call.startsWith("queued-prompt:")), parsedEvent.name)
  }
})

test("cli stdin key controller lets command center own prompt input", () => {
  const harness = createHarness({
    promptFocused: true,
    commandCenterOpen: true,
    parsedEvent: keyEvent("escape"),
  })

  assert.equal(harness.controller.handleData("x"), true)
  assert.deepEqual(harness.calls(), [
    "parse:x",
    "session-browser:escape",
    "focused:escape",
    "clear-command-center",
  ])
})

test("cli stdin key controller routes workspace and focus cycling shortcuts", () => {
  const workspaceHarness = createHarness({ parsedEvent: keyEvent("p", { ctrl: true }) })
  assert.equal(workspaceHarness.controller.handleData("x"), true)
  assert.deepEqual(workspaceHarness.calls(), [
    "parse:x",
    "session-browser:p",
    "focused:p",
    "toggle-workspace",
  ])

  const focusHarness = createHarness({ attached: true, parsedEvent: keyEvent("tab") })
  assert.equal(focusHarness.controller.handleData("x"), true)
  assert.deepEqual(focusHarness.calls(), [
    "parse:x",
    "session-browser:tab",
    "focused:tab",
    "cycle-agent-focus",
  ])

  const workflowHarness = createHarness({
    attached: true,
    workflowScreenActive: true,
    parsedEvent: keyEvent("tab"),
  })
  assert.equal(workflowHarness.controller.handleData("x"), true)
  assert.deepEqual(workflowHarness.calls(), [
    "parse:x",
    "session-browser:tab",
    "focused:tab",
    "cycle-workflow-node",
  ])
})

test("cli stdin key controller routes copy and ctrl-c shortcuts", () => {
  const copyHarness = createHarness({
    copyHandled: true,
    parsedEvent: keyEvent("c", { meta: true }),
  })
  assert.equal(copyHarness.controller.handleData("x"), true)
  assert.deepEqual(copyHarness.calls(), [
    "parse:x",
    "session-browser:c",
    "copy",
  ])

  const stopHarness = createHarness({
    activeTurnWork: true,
    parsedEvent: keyEvent("c", { ctrl: true }),
  })
  assert.equal(stopHarness.controller.handleData("x"), true)
  assert.deepEqual(stopHarness.calls(), [
    "parse:x",
    "session-browser:c",
    "focused:c",
    "stop",
  ])

  const exitHarness = createHarness({ parsedEvent: keyEvent("c", { ctrl: true }) })
  assert.equal(exitHarness.controller.handleData("x"), true)
  assert.deepEqual(exitHarness.calls(), [
    "parse:x",
    "session-browser:c",
    "focused:c",
    "exit",
  ])
})

test("cli stdin key controller routes prompt attachment edits", () => {
  const editHarness = createHarness({
    promptFocused: true,
    removeEditHandled: true,
    parsedEvent: keyEvent("backspace"),
  })
  assert.equal(editHarness.controller.handleData("x"), true)
  assert.deepEqual(editHarness.calls(), [
    "parse:x",
    "session-browser:backspace",
    "focused:backspace",
    "remove-edit:backspace",
  ])

  const trailingHarness = createHarness({
    attached: true,
    currentPromptText: "",
    pendingAttachmentCount: 1,
    parsedEvent: keyEvent("backspace"),
  })
  assert.equal(trailingHarness.controller.handleData("x"), true)
  assert.deepEqual(trailingHarness.calls(), [
    "parse:x",
    "session-browser:backspace",
    "focused:backspace",
    "remove-last-attachment",
  ])
})

test("cli stdin key controller routes workflow detail pane keys before prompt navigation", () => {
  const harness = createHarness({
    workflowScreenActive: true,
    workflowDetailHandled: true,
    parsedEvent: keyEvent("l"),
  })

  assert.equal(harness.controller.handleData("x"), true)
  assert.deepEqual(harness.calls(), [
    "parse:x",
    "session-browser:l",
    "focused:l",
    "workflow-detail:l",
  ])
})

test("cli stdin key controller falls through to prompt-turn and waiting-room handlers", () => {
  const promptTurnHarness = createHarness({
    promptTurnHandled: true,
    parsedEvent: keyEvent("up", { shift: true }),
  })
  assert.equal(promptTurnHarness.controller.handleData("x"), true)
  assert.deepEqual(promptTurnHarness.calls(), [
    "parse:x",
    "session-browser:up",
    "focused:up",
    "prompt-turn:up",
  ])

  const waitingRoomHarness = createHarness({
    waitingRoomHandled: true,
    parsedEvent: keyEvent("down"),
  })
  assert.equal(waitingRoomHarness.controller.handleData("x"), true)
  assert.deepEqual(waitingRoomHarness.calls(), [
    "parse:x",
    "session-browser:down",
    "focused:down",
    "prompt-turn:down",
    "waiting-room:down",
  ])
})

test("MP-08/MP-11 Meta+C copies a selection before queued-prompt cancel and cancels only without one", () => {
  const copyHarness = createHarness({ parsedEvent: keyEvent("c", { meta: true }), queuedPromptHandled: true, copyHandled: true })
  assert.equal(copyHarness.controller.handleData("x"), true)
  assert.ok(copyHarness.calls().includes("copy"))
  assert.ok(!copyHarness.calls().includes("queued-prompt:c"))

  const cancelHarness = createHarness({ parsedEvent: keyEvent("c", { meta: true }), queuedPromptHandled: true, copyHandled: false })
  assert.equal(cancelHarness.controller.handleData("x"), true)
  assert.ok(cancelHarness.calls().includes("queued-prompt:c"))
})

test("MP-08/MP-11 Ctrl+Shift+C never stops the provider or exits when selection is empty", () => {
  for (const copyHandled of [true, false]) {
    const harness = createHarness({ parsedEvent: keyEvent("c", { ctrl: true, shift: true }), activeTurnWork: true, copyHandled })
    assert.equal(harness.controller.handleData("x"), true)
    assert.ok(harness.calls().includes("copy"))
    assert.ok(!harness.calls().includes("stop"))
    assert.ok(!harness.calls().includes("exit"))
  }
})

test("MP-08 / MP-10 F6 copies without kitty keyboard support and never stops or exits", () => {
  for (const copyHandled of [true, false]) {
    for (const activeTurnWork of [true, false]) {
      let clears = 0
      const harness = createHarness({ parsedEvent: keyEvent("f6"), copyHandled, activeTurnWork, clearTextSelection: () => { clears++ } })
      assert.equal(harness.controller.handleData("\x1b[17~"), true)
      assert.ok(harness.calls().includes("copy"))
      assert.ok(!harness.calls().includes("stop"))
      assert.ok(!harness.calls().includes("exit"))
      assert.equal(clears, 0)
    }
  }
})

for (const activeTurnWork of [true, false]) test(`workflow composer preserves Ctrl+C ${activeTurnWork ? "stop" : "exit"}`, () => {
  const harness = createHarness({ roomWorkflowInput: true, activeTurnWork, parsedEvent: keyEvent("c", { ctrl: true }) })
  assert.equal(harness.controller.handleData("\u0003"), true)
  assert.equal(harness.calls().at(-1), activeTurnWork ? "stop" : "exit")
})

test("workflow composer excludes agent navigation and editing shortcuts", () => {
  for (const event of [keyEvent("p", { ctrl: true }), keyEvent("tab"), keyEvent("backspace")]) {
    const harness = createHarness({ roomWorkflowInput: true, attached: true, promptFocused: true, parsedEvent: event })
    assert.equal(harness.controller.handleData("x"), true)
    assert.ok(!harness.calls().some(call => /toggle-workspace|cycle-agent|remove-edit|prompt-turn/.test(call)))
  }
})

function createHarness(options: {
  roomWorkflowInput?: boolean
  parsedEvent?: CliStdinKeyEvent | null
  handleProjectEnvironmentKey?: (event: CliStdinKeyEvent) => boolean
  clearTextSelection?: () => void
  dialogOpen?: boolean
  sessionBrowserHandled?: boolean
  focusedInteractionActive?: boolean
  focusedInteractionHandled?: boolean
  queuedPromptHandled?: boolean
  onQueuedPromptKey?: (event: CliStdinKeyEvent) => boolean
  promptFocused?: boolean
  commandCenterOpen?: boolean
  commandCenterQuery?: string
  attached?: boolean
  workflowScreenActive?: boolean
  copyHandled?: boolean
  activeTurnWork?: boolean
  removeEditHandled?: boolean
  currentPromptText?: string
  pendingAttachmentCount?: number
  promptTurnHandled?: boolean
  waitingRoomHandled?: boolean
  workflowDetailHandled?: boolean
} = {}) {
  const calls: string[] = []
  const parsedEvent = options.parsedEvent === undefined
    ? keyEvent("x")
    : options.parsedEvent
  const deps: CliStdinKeyControllerDeps = {
    roomWorkflowsOwnsInput: () => options.roomWorkflowInput ?? false,
    ...(options.clearTextSelection ? { clearTextSelection: options.clearTextSelection } : {}),
    createStdinParser: () => ({
      push: (data) => {
        calls.push(`parse:${Buffer.from(data).toString()}`)
      },
      drain: (onEvent) => {
        if (parsedEvent) onEvent({ type: "key", key: parsedEvent })
      },
    }),
    ...(options.handleProjectEnvironmentKey ? { handleProjectEnvironmentKey: options.handleProjectEnvironmentKey } : {}),
    dialogOverlayOpen: () => options.dialogOpen ?? false,
    closeActiveDialogOverlay: () => {
      calls.push("close-dialog")
    },
    handleSessionBrowserKey: (event) => {
      calls.push(`session-browser:${event.name}`)
      return options.sessionBrowserHandled ?? false
    },
    requestExit: () => {
      calls.push("exit")
    },
    focusedInteractionActive: () => options.focusedInteractionActive ?? false,
    handleFocusedInteractionKey: (event) => {
      calls.push(`focused:${event.name}`)
      return options.focusedInteractionHandled ?? false
    },
    handleQueuedPromptKey: (event) => {
      calls.push(`queued-prompt:${event.name}`)
      if (options.onQueuedPromptKey) {
        return options.onQueuedPromptKey(event)
      }
      return options.queuedPromptHandled ?? false
    },
    promptFocused: () => options.promptFocused ?? false,
    commandCenterOpen: () => options.commandCenterOpen ?? false,
    commandCenterQuery: () => options.commandCenterQuery ?? "",
    clearCommandCenter: () => {
      calls.push("clear-command-center")
    },
    toggleWorkspaceScreen: () => {
      calls.push("toggle-workspace")
    },
    toggleTranscriptDetails: () => {
      calls.push("toggle-details")
    },
    isAttached: () => options.attached ?? false,
    workflowScreenActive: () => options.workflowScreenActive ?? false,
    cycleWorkflowCanvasNode: () => {
      calls.push("cycle-workflow-node")
    },
    handleWorkflowDetailPaneKey: (event) => {
      calls.push(`workflow-detail:${event.name}`)
      return options.workflowDetailHandled ?? false
    },
    cycleAgentFocus: () => {
      calls.push("cycle-agent-focus")
    },
    copyPromptSelection: () => {
      calls.push("copy")
      return options.copyHandled ?? false
    },
    hasActiveTurnWork: () => options.activeTurnWork ?? false,
    requestPromptStop: () => {
      calls.push("stop")
    },
    removePromptAttachmentsForEdit: (edit) => {
      calls.push(`remove-edit:${edit}`)
      return options.removeEditHandled ?? false
    },
    currentPromptText: () => options.currentPromptText ?? "prompt",
    pendingAttachmentCount: () => options.pendingAttachmentCount ?? 0,
    removeLastPendingPromptAttachment: () => {
      calls.push("remove-last-attachment")
    },
    handlePromptTurnNavigationKey: (event) => {
      calls.push(`prompt-turn:${event.name}`)
      return options.promptTurnHandled ?? false
    },
    handleWaitingRoomKey: (event) => {
      calls.push(`waiting-room:${event.name}`)
      return options.waitingRoomHandled ?? false
    },
  }
  return {
    controller: createCliStdinKeyController(deps),
    calls: () => calls,
  }
}

// MP-08 / MP-10 (panes R11, PANE-05/D15): with app mouse off by default, Ctrl+O
// is the keyboard path to open every tool call and thought of the pane.
test("cli stdin key controller toggles transcript details on Ctrl+O when attached", () => {
  const attached = createHarness({ parsedEvent: keyEvent("o", { ctrl: true }), attached: true })
  assert.equal(attached.controller.handleData("x"), true)
  assert.equal(attached.calls().includes("toggle-details"), true)

  const detached = createHarness({ parsedEvent: keyEvent("o", { ctrl: true }), attached: false })
  detached.controller.handleData("x")
  assert.equal(detached.calls().includes("toggle-details"), false)
})

function keyEvent(name: string, options: {
  eventType?: string
  ctrl?: boolean
  meta?: boolean
  alt?: boolean
  shift?: boolean
} = {}): CliStdinKeyEvent {
  const event: CliStdinKeyEvent = { name }
  if (options.eventType !== undefined) {
    event.eventType = options.eventType
  }
  if (options.ctrl !== undefined) {
    event.ctrl = options.ctrl
  }
  if (options.meta !== undefined) {
    event.meta = options.meta
  }
  if (options.alt !== undefined) {
    event.alt = options.alt
  }
  if (options.shift !== undefined) {
    event.shift = options.shift
  }
  return event
}

// MP-08/MP-10: exercise the real stdin ordering with the mounted read controller.
test("Environment panel preserves Ctrl+E and Ctrl+C exit/stop through stdin", async () => {
  for (const name of ["e", "c"]) for (const activeTurnWork of [false, true]) {
    const panel = createProjectEnvironmentReadController({ send: async () => { throw new Error("read only") }, render() {}, pageSize: () => 10, textWidth: () => 72 })
    await panel.open("project")
    const harness = createHarness({ parsedEvent: keyEvent(name, { ctrl: true }), activeTurnWork, handleProjectEnvironmentKey: panel.handleKey })
    assert.equal(harness.controller.handleData("x"), true)
    assert(harness.calls().includes(name === "c" && activeTurnWork ? "stop" : "exit"))
  }
})

 test("MP-08 / MP-10 transcript copy precedes both active interaction paths", () => {
  for (const promptFocused of [true, false]) for (const event of [keyEvent("f6"), keyEvent("c", {meta: true}), keyEvent("c", {ctrl: true, shift: true})]) {
    const h = createHarness({parsedEvent: event, promptFocused, focusedInteractionActive: true, focusedInteractionHandled: true, copyHandled: true})
    h.controller.handleData("x")
    assert.ok(h.calls().includes("copy"))
    assert.ok(!h.calls().includes(`focused:${event.name}`))
  }
})
