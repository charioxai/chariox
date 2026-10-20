import assert from "node:assert/strict"
import test from "node:test"

import { createPromptAttachmentController } from "./prompt-attachment-controller.js"
import { handleAppSlashCommand } from "./app-command-handler.js"
import { sharedShellCommandForSlashCommand } from "./commands.js"
import {
  createSlashCommandSubmitController,
  type SlashCommandSubmitControllerDeps,
} from "./slash-command-submit-controller.js"

test("MP-08/MP-10/MP-11 a delayed slice command preserves the next agent spawn draft", async () => {
  let draft = "/slice start own-slice"
  let finish!: () => void
  const pending = new Promise<void>(resolve => { finish = resolve })
  const harness = createHarness({ attached: true, clearPromptText: () => { draft = "" } })
  Object.assign(harness.deps, { getPromptText: () => draft })
  harness.deps.handleSliceCommand = () => pending
  const controller = createSlashCommandSubmitController(harness.deps)
  const submitted = controller.submit(draft, { allowSlashCommandSubmission: true })
  await Promise.resolve()
  draft = "/agent spawn leased-editor opencode-go/gpt-5.6-luna --slice own-slice"
  finish()
  assert.equal((await submitted)?.kind, "slice")
  assert.equal(draft, "/agent spawn leased-editor opencode-go/gpt-5.6-luna --slice own-slice")
  // The submitted command is consumed before awaiting its handler; the new draft survives.
  assert.equal(harness.clearPromptCount(), 1)
  assert.equal(harness.commandCenterClearCount(), 1)
  assert.deepEqual(harness.promptHistoryIndexes(), [null])
})

test("slash command submit records attached history and clears selector-backed command UI", async () => {
  const harness = createHarness({ attached: true })
  const controller = createSlashCommandSubmitController(harness.deps)

  const command = await controller.submit("/model opencode/gpt-5.4", {
    allowSlashCommandSubmission: true,
  })

  assert.equal(command?.kind, "model")
  assert.deepEqual(harness.calls(), ["model:opencode/gpt-5.4"])
  assert.deepEqual(harness.recordedHistory(), [{ sessionId: "session-1", rawPrompt: "/model opencode/gpt-5.4" }])
  assert.equal(harness.clearPromptCount(), 1)
  assert.deepEqual(harness.promptHistoryIndexes(), [null])
  assert.deepEqual(harness.promptHistoryDrafts(), [null])
  assert.equal(harness.commandCenterClearCount(), 1)
})

test("slash command submit bypasses slash commands when submission is not allowed", async () => {
  const harness = createHarness({ attached: true })
  const controller = createSlashCommandSubmitController(harness.deps)

  const command = await controller.submit("/model opencode/gpt-5.4", {
    allowSlashCommandSubmission: false,
  })

  assert.equal(command, null)
  assert.deepEqual(harness.calls(), [])
  assert.deepEqual(harness.recordedHistory(), [])
  assert.equal(harness.clearPromptCount(), 0)
})

test("slash command submit keeps command center for session commands", async () => {
  const harness = createHarness({ attached: true })
  const controller = createSlashCommandSubmitController(harness.deps)

  const command = await controller.submit("/session list", {
    allowSlashCommandSubmission: true,
  })

  assert.equal(command?.kind, "session")
  assert.deepEqual(harness.calls(), ["session:list"])
  assert.equal(harness.clearPromptCount(), 1)
  assert.equal(harness.commandCenterClearCount(), 0)
})

test("slash command submit dispatches agent wait commands", async () => {
  const harness = createHarness({ attached: true })
  const controller = createSlashCommandSubmitController(harness.deps)

  const command = await controller.submit("/wait-every 2 Review health", {
    allowSlashCommandSubmission: true,
  })

  assert.equal(command?.kind, "wait")
  assert.deepEqual(harness.calls(), ["wait:recurring:2:Review health"])
  assert.equal(harness.clearPromptCount(), 1)
})

test("App slash commands work while no session is attached", async () => {
  const harness = createHarness()
  harness.deps.handleAppCommand = command => { harness.calls().push(`app:${command.args.join(" ")}`) }
  const controller = createSlashCommandSubmitController(harness.deps)
  const command = await controller.submit("/app journal install-1", { allowSlashCommandSubmission: true })
  assert.equal(command?.kind, "app")
  assert.deepEqual(harness.calls(), ["app:journal install-1"])
  assert.deepEqual(harness.recordedHistory(), [])
  assert.equal(harness.commandCenterClearCount(), 1)
})

for (const [label, raw, request] of [
  ["a bare double-quoted JSON inbox payload", '/app inbox test todo mail occ-1 {"step":"request","n":1}',
    { TestAppInboxRoute: { installation_id: "todo", route_id: "mail", occurrence_id: "occ-1", payload: { step: "request", n: 1 } } }],
  ["a single-quoted inbox payload with spaces", `/app inbox test todo mail occ-1 '{"title":"a  b"}'`,
    { TestAppInboxRoute: { installation_id: "todo", route_id: "mail", occurrence_id: "occ-1", payload: { title: "a  b" } } }],
  ["a quoted route ID with spaces and a single-quoted payload", `/app inbox test todo "mail route" occ-1 '{"title":"a  b"}'`,
    { TestAppInboxRoute: { installation_id: "todo", route_id: "mail route", occurrence_id: "occ-1", payload: { title: "a  b" } } }],
  ["a copied next-page command", '/app list --after "todo"', { ListAppInstallations: { after: "todo", limit: null } }],
] as const) {
  test(`${label} reaches the kernel through the TUI's App handler, not the shared shell`, async () => {
    const harness = createHarness({
      attached: true,
      // The TUI's real routing: only lines the shared shell owns go there.
      handleSharedShellCommand: async (command) => {
        if (sharedShellCommandForSlashCommand(command) === null) return false
        harness.sharedCommands().push(command)
        return true
      },
    })
    const requests: unknown[] = []
    harness.deps.handleAppCommand = command => handleAppSlashCommand({
      sendAppRequest: async (sent) => {
        requests.push(sent)
        return "TestAppInboxRoute" in sent
          ? { AppInboxOccurrenceAccepted: { installation_id: "todo", route_id: "mail", occurrence_id: "occ-1", duplicate: false } }
          : { AppInstallationsListed: { installations: [], next_cursor: null } }
      },
      appendNotice: () => {},
      flashFooter: message => assert.fail(message),
    }, command)
    const controller = createSlashCommandSubmitController(harness.deps)

    const command = await controller.submit(raw, { allowSlashCommandSubmission: true })

    assert.equal(command?.kind, "app")
    assert.deepEqual(harness.sharedCommands(), [])
    assert.deepEqual(requests, [request])
    assert.deepEqual(harness.footerMessages(), [])
  })
}

test("slash command submit dispatches Room environment commands", async () => {
  const harness = createHarness({ attached: true })
  const controller = createSlashCommandSubmitController(harness.deps)

  const command = await controller.submit("/room status", {
    allowSlashCommandSubmission: true,
  })

  assert.equal(command?.kind, "room")
  assert.deepEqual(harness.calls(), ["room:status"])
  assert.equal(harness.clearPromptCount(), 1)
  assert.equal(harness.commandCenterClearCount(), 1)
})

test("slash command submit delegates catalog-only kernel commands to shared shell", async () => {
  const harness = createHarness({
    attached: true,
    handleSharedShellCommand: async (command) => {
      harness.sharedCommands().push(command)
      return true
    },
  })
  const controller = createSlashCommandSubmitController(harness.deps)

  const command = await controller.submit("/workflow trigger list", {
    allowSlashCommandSubmission: true,
  })

  assert.equal(command?.kind, "workflow")
  assert.deepEqual(harness.sharedCommands(), ["/workflow trigger list"])
  assert.deepEqual(harness.calls(), [])
  assert.equal(harness.clearPromptCount(), 1)
})

test("slash command submit clears the exit command before requesting exit", async () => {
  const events: string[] = []
  const harness = createHarness({
    clearPromptText: () => events.push("clear"),
    onExit: () => events.push("exit"),
  })
  const controller = createSlashCommandSubmitController(harness.deps)

  const command = await controller.submit("/exit", {
    allowSlashCommandSubmission: true,
  })

  assert.equal(command?.kind, "exit")
  assert.deepEqual(events, ["clear", "exit"])
  assert.equal(harness.clearPromptCount(), 1)
})

test("slash command submit reports unknown session commands", async () => {
  const harness = createHarness({
    handleSessionCommand: () => false,
  })
  const controller = createSlashCommandSubmitController(harness.deps)

  await controller.submit("/session mystery", {
    allowSlashCommandSubmission: true,
  })

  assert.equal(harness.footerMessages().at(-1)?.message, "unknown /session command")
  assert.equal(harness.clearPromptCount(), 1)
})

test("slash command submit logs attachment command failures", async () => {
  const harness = createHarness({
    handleAttachmentCommand: () => {
      throw new Error("attach failed")
    },
  })
  const controller = createSlashCommandSubmitController(harness.deps)

  await controller.submit("/attach missing.txt", {
    allowSlashCommandSubmission: true,
    trimmedPrompt: "/attach missing.txt",
  })

  assert.equal(harness.footerMessages().at(-1)?.message, "attach failed")
  assert.equal(harness.logErrors().at(-1)?.message, "attachment command failed")
  assert.equal(harness.logErrors().at(-1)?.fields.command, "/attach missing.txt")
  assert.equal(harness.clearPromptCount(), 1)
})

for (const outcome of ["success", "error", "shared shell"] as const) {
  test(`a delayed ${outcome} command preserves the next prompt draft and selector`, async () => {
    const submitted = "/agent account ctx kaissandra.ai"
    let text = submitted
    let complete!: () => void
    let started!: () => void
    const pending = new Promise<void>(resolve => { complete = resolve })
    const entered = new Promise<void>(resolve => { started = resolve })
    const harness = createHarness({ clearPromptText: () => { text = "" } })
    const handler = async () => {
      started()
      await pending
      if (outcome === "error") throw new Error("account failed")
    }
    if (outcome === "shared shell") {
      harness.deps.handleSharedShellCommand = async () => { await handler(); return true }
    } else {
      harness.deps.handleAgentCommand = handler
    }
    const controller = createSlashCommandSubmitController(harness.deps)
    const result = controller.submit(submitted, { allowSlashCommandSubmission: true })
    await entered
    // MP-08 / MP-10 / MP-11: main consumes the submitted command before
    // awaiting the handler. Completion must not clear the later draft again.
    assert.equal(harness.clearPromptCount(), 1)
    assert.equal(harness.commandCenterClearCount(), 1)
    text = "/agent model ctx gpt-6-luna"
    complete()
    assert.equal((await result)?.kind, "agent")
    assert.equal(text, "/agent model ctx gpt-6-luna")
    assert.equal(harness.clearPromptCount(), 1)
    assert.equal(harness.commandCenterClearCount(), 1)
    assert.deepEqual(harness.promptHistoryIndexes(), [null])
    assert.deepEqual(harness.promptHistoryDrafts(), [null])
    if (outcome === "error") assert.equal(harness.footerMessages().at(-1)?.message, "account failed")
  })
}

test("a completed command clears its unchanged submitted draft", async () => {
  let text = "/agent account ctx kaissandra.ai"
  const harness = createHarness({ clearPromptText: () => { text = "" } })
  await createSlashCommandSubmitController(harness.deps).submit(text, { allowSlashCommandSubmission: true })
  assert.equal(text, "")
  assert.equal(harness.clearPromptCount(), 1)
  assert.equal(harness.commandCenterClearCount(), 1)
  assert.deepEqual(harness.promptHistoryIndexes(), [null])
  assert.deepEqual(harness.promptHistoryDrafts(), [null])
})

function createHarness(options: {
  attached?: boolean
  clearPromptText?: () => void
  onExit?: SlashCommandSubmitControllerDeps["onExit"]
  handleAttachmentCommand?: SlashCommandSubmitControllerDeps["handleAttachmentCommand"]
  handleSessionCommand?: SlashCommandSubmitControllerDeps["handleSessionCommand"]
  handleSharedShellCommand?: SlashCommandSubmitControllerDeps["handleSharedShellCommand"]
} = {}) {
  const calls: string[] = []
  const recordedHistory: Array<{ sessionId: string; rawPrompt: string }> = []
  const promptHistoryIndexes: Array<number | null> = []
  const promptHistoryDrafts: Array<string | null> = []
  const footerMessages: Array<{ message: string; tone: "info" | "error" }> = []
  const logErrors: Array<{ message: string; fields: Record<string, unknown> }> = []
  const sharedCommands: string[] = []
  let clearPromptCount = 0
  let commandCenterClearCount = 0

  const deps: SlashCommandSubmitControllerDeps = {
    isAttached: () => options.attached ?? false,
    getSessionId: () => "session-1",
    recordPromptAreaHistoryEntry: (sessionId, rawPrompt) => {
      recordedHistory.push({ sessionId, rawPrompt })
    },
    clearPromptText: () => {
      clearPromptCount += 1
      options.clearPromptText?.()
    },
    setPromptHistoryIndex: (index) => {
      promptHistoryIndexes.push(index)
    },
    setPromptHistoryDraft: (draft) => {
      promptHistoryDrafts.push(draft)
    },
    clearCommandCenter: () => {
      commandCenterClearCount += 1
    },
    flashFooter: (message, tone) => {
      footerMessages.push({ message, tone })
    },
    logError: (message, fields) => {
      logErrors.push({ message, fields })
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
    onExit: options.onExit ?? (() => calls.push("exit")),
    onWaiting: () => calls.push("waiting"),
    onApprovals: () => calls.push("approvals"),
    onStop: () => calls.push("stop"),
    handleAttachmentCommand: options.handleAttachmentCommand ?? ((raw) => calls.push(`attachment:${raw}`)),
    handleSessionCommand: options.handleSessionCommand ?? ((command) => {
      calls.push(`session:${command.action ?? ""}`)
      return true
    }),
    handleProviderCommand: (command) => calls.push(`provider:${command.value}`),
    handleModelCommand: (command) => calls.push(`model:${command.value}`),
    handleVariantCommand: (command) => calls.push(`variant:${command.value}`),
    handleModeCommand: (command) => calls.push(`mode:${command.value}`),
    handlePermissionsCommand: (command) => calls.push(`permissions:${command.value}`),
    handleViewCommand: (command) => calls.push(`view:${command.value}`),
    handleUndoCommand: (command) => calls.push(`undo:${command.args.join(" ")}`),
    handleForkCommand: (command) => calls.push(`fork:${command.args.join(" ")}`),
    handleAgentCommand: (command) => calls.push(`agent:${command.args.join(" ")}`),
    handleKernelCommand: (command) => calls.push(`kernel:${command.args.join(" ")}`),
    handleMachineCommand: (command) => calls.push(`machine:${command.args.join(" ")}`),
    handleSliceCommand: (command) => calls.push(`slice:${command.args.join(" ")}`),
    handleRoomCommand: (command) => calls.push(`room:${command.args.join(" ")}`),
    handleRelayCommand: (command) => calls.push(`relay:${command.args.join(" ")}`),
    handleCloudCommand: (command) => calls.push(`cloud:${command.args.join(" ")}`),
    handleCollabCommand: (command) => calls.push(`collab:${command.args.join(" ")}`),
    handleConfigCommand: (command) => calls.push(`config:${command.args.join(" ")}`),
    handleWorkspaceCommand: (command) => calls.push(`workspace:${command.args.join(" ")}`),
    handleWorktreeCommand: (command) => calls.push(`worktree:${command.args.join(" ")}`),
    handleWorkflowCommand: (command) => calls.push(`workflow:${command.args.join(" ")}`),
    handleLoopCommand: (command) => calls.push(`loop:${command.prompt}`),
    handleGoalCommand: (command) => calls.push(`goal:${command.prompt}`),
    handleWaitCommand: (command) => calls.push(`wait:${command.scheduleKind}:${command.minutes}:${command.prompt}`),
    handleMcpCommand: (command) => calls.push(`mcp:${command.args.join(" ")}`),
    handleSkillCommand: (command) => calls.push(`skill:${command.args.join(" ")}`),
    handleEnvCommand: (command) => calls.push(`env:${command.args.join(" ")}`),
    handleScriptCommand: (command) => calls.push(`script:${command.args.join(" ")}`),
    handleCredentialCommand: (command) => calls.push(`credential:${command.args.join(" ")}`),
    handleConnectorCommand: (command) => calls.push(`connector:${command.args.join(" ")}`),
    handleExtensionCommand: (command) => calls.push(`extension:${command.args.join(" ")}`),
    ...(options.handleSharedShellCommand ? { handleSharedShellCommand: options.handleSharedShellCommand } : {}),
  }

  return {
    deps,
    calls: () => calls,
    recordedHistory: () => recordedHistory,
    promptHistoryIndexes: () => promptHistoryIndexes,
    promptHistoryDrafts: () => promptHistoryDrafts,
    footerMessages: () => footerMessages,
    logErrors: () => logErrors,
    sharedCommands: () => sharedCommands,
    clearPromptCount: () => clearPromptCount,
    commandCenterClearCount: () => commandCenterClearCount,
  }
}

test("/approvals clears command UI before opening and bypasses shared shell execution", async () => {
  const h = createHarness({ attached: true })
  let opened = 0
  const controller = createSlashCommandSubmitController({
    ...h.deps,
    handleSharedShellCommand: async () => { assert.fail("approvals sent to shell") },
    onApprovals: () => {
      assert.equal(h.clearPromptCount(), 1)
      assert.equal(h.commandCenterClearCount(), 1)
      opened += 1
    },
  })
  assert.equal((await controller.submit("/approvals", { allowSlashCommandSubmission: true }))?.kind, "approvals")
  assert.equal(opened, 1)
})

// MP-08/MP-10/MP-11: routing must reach the real shared executor, not alias handling.
test("slash command submit sends workflow notification settings to shared shell", async () => {
  const harness = createHarness({
    attached: true,
    handleSharedShellCommand: async (command) => {
      harness.sharedCommands().push(command)
      return true
    },
  })
  const controller = createSlashCommandSubmitController(harness.deps)
  await controller.submit("/workflow notifications on retained-source", {
    allowSlashCommandSubmission: true,
  })
  assert.deepEqual(harness.sharedCommands(), ["/workflow notifications on retained-source"])
  assert.deepEqual(harness.calls(), [])
  assert.equal(harness.clearPromptCount(), 1)
})

test("attachment commands clear their command text before intake inserts image tokens",async()=>{
  const events:string[]=[]
  const harness=createHarness({clearPromptText:()=>events.push("clear"),handleAttachmentCommand:()=>{events.push("image-token")}})
  await createSlashCommandSubmitController(harness.deps).submit("/attach region tab t 0 0 1 1",{allowSlashCommandSubmission:true})
  assert.deepEqual(events,["clear","image-token"])
})

test("MP-08 / MP-11 attachment command preserves its newly inserted file token", async () => {
  const h = createHarness({ attached: true })
  let text = "/attach README.md"
  let pending: import("./prompt-attachment-state.js").PendingPromptAttachment[] = []
  const attachments = createPromptAttachmentController({
    getPromptInput: () => null,
    getPromptText: () => text,
    setPromptText: value => { text = value },
    pendingAttachments: () => pending,
    setPendingAttachments: value => { pending = value },
    updatePendingAttachments: update => { pending = update(pending) },
    refreshHighlights() {}, updateSessionChrome() {}, requestRender() {},
  })
  const controller = createSlashCommandSubmitController({
    ...h.deps,
    clearPromptText: () => { text = ""; attachments.syncFromText(text) },
    handleAttachmentCommand: async () => {
      await Promise.resolve()
      attachments.addStoredFiles([{ id: "readme", url: "file:///public/README.md", mime: "text/markdown", filename: "README.md", kind: "text" }], 0)
    },
  })
  assert.equal((await controller.submit("/attach README.md", { allowSlashCommandSubmission: true }))?.kind, "attachment")
  assert.equal(text, "[file 1]")
  assert.equal(pending[0]?.filename, "README.md")
})

for (const rejects of [false, true]) {
  test(`MP-08 / MP-10 / MP-11 pending focus preserves the next draft (${rejects ? "error" : "success"})`, async () => {
    let prompt = "/agent focus reviewer"
    let settle!: () => void
    const pending = new Promise<void>(resolve => { settle = resolve })
    const h = createHarness({ attached: true, clearPromptText: () => { prompt = "" } })
    h.deps.handleAgentCommand = async () => {
      await pending
      if (rejects) throw new Error("focus unavailable")
    }
    const controller = createSlashCommandSubmitController(h.deps)
    const submission = controller.submit(prompt, { allowSlashCommandSubmission: true })
    prompt = "Read the public README next."
    settle()
    await submission
    assert.equal(prompt, "Read the public README next.")
    assert.equal(h.clearPromptCount(), 1)
    assert.equal(h.commandCenterClearCount(), 1)
  })
}
