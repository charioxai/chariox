import assert from "node:assert/strict"
import test from "node:test"

import {
  createSlashCommandSubmitController,
  type SlashCommandSubmitControllerDeps,
} from "./slash-command-submit-controller.js"

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

test("slash command submit delegates catalog-only kernel commands to shared shell", async () => {
  const harness = createHarness({
    attached: true,
    handleSharedShellCommand: async (command) => {
      harness.sharedCommands().push(command)
      return true
    },
  })
  const controller = createSlashCommandSubmitController(harness.deps)

  const command = await controller.submit("/workflow publication list", {
    allowSlashCommandSubmission: true,
  })

  assert.equal(command?.kind, "workflow")
  assert.deepEqual(harness.sharedCommands(), ["/workflow publication list"])
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
