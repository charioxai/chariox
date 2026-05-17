import assert from "node:assert/strict"
import test from "node:test"

import {
  createPromptSubmitCoordinator,
  type PromptSubmitCoordinatorDeps,
} from "./prompt-submit-coordinator.js"

test("prompt submit coordinator is idle without prompt text", async () => {
  const harness = createHarness({ promptText: null })

  await harness.coordinator.submit()

  assert.equal(harness.ensureCount(), 0)
  assert.deepEqual(harness.calls(), [])
})

test("prompt submit coordinator clears empty prompts without attachments", async () => {
  const harness = createHarness({ promptText: "   " })

  await harness.coordinator.submit()

  assert.equal(harness.clearCount(), 1)
  assert.deepEqual(harness.calls(), ["ensure"])
})

test("prompt submit coordinator routes workspace shell commands and clears input", async () => {
  const harness = createHarness({ promptText: "@ status", workflowScreen: true })

  await harness.coordinator.submit()

  assert.deepEqual(harness.calls(), ["ensure", "workspace:@ status"])
  assert.equal(harness.clearCount(), 1)
})

test("prompt submit coordinator reports workspace shell failures and clears input", async () => {
  const harness = createHarness({
    promptText: "@ status",
    workflowScreen: true,
    submitWorkspaceShellCommand: async () => {
      throw new Error("shell failed")
    },
  })

  await harness.coordinator.submit()

  assert.equal(harness.footerMessages().at(-1)?.message, "shell failed")
  assert.equal(harness.clearCount(), 1)
})

test("prompt submit coordinator blocks prompt text while instructions editor is open", async () => {
  const harness = createHarness({ promptText: "new instructions", instructionsEditorOpen: true })

  await harness.coordinator.submit()

  assert.equal(harness.footerMessages().at(-1)?.tone, "info")
  assert.equal(harness.clearCount(), 1)
  assert.deepEqual(harness.calls(), ["ensure"])
})

test("prompt submit coordinator lets handled slash commands stop routing", async () => {
  const harness = createHarness({ promptText: "/session list", slashHandled: true })

  await harness.coordinator.submit()

  assert.deepEqual(harness.slashCalls(), [{
    rawPrompt: "/session list",
    allowSlashCommandSubmission: true,
    trimmedPrompt: "/session list",
  }])
  assert.deepEqual(harness.calls(), ["ensure", "slash:/session list"])
})

test("prompt submit coordinator allows slash commands on workflow screen", async () => {
  const harness = createHarness({ promptText: "/session list", workflowScreen: true, slashHandled: true })

  await harness.coordinator.submit()

  assert.equal(harness.slashCalls().at(-1)?.allowSlashCommandSubmission, true)
})

test("prompt submit coordinator routes provider namespace prompts before attached checks", async () => {
  const harness = createHarness({ promptText: "@codex hello", providerHandled: true, attached: false })

  await harness.coordinator.submit()

  assert.deepEqual(harness.calls(), ["ensure", "slash:@codex hello", "provider:@codex hello"])
  assert.deepEqual(harness.footerMessages(), [])
})

test("prompt submit coordinator reports detached normal prompts", async () => {
  const harness = createHarness({ promptText: "hello", attached: false })

  await harness.coordinator.submit()

  assert.equal(harness.footerMessages().at(-1)?.tone, "error")
  assert.equal(harness.clearCount(), 1)
})

test("prompt submit coordinator routes attached workflow and normal prompts", async () => {
  const workflowHarness = createHarness({ promptText: "run workflow", workflowScreen: true })
  await workflowHarness.coordinator.submit()
  assert.deepEqual(workflowHarness.calls(), ["ensure", "slash:run workflow", "provider:run workflow", "workflow:run workflow"])

  const normalHarness = createHarness({ promptText: "hello" })
  await normalHarness.coordinator.submit()
  assert.deepEqual(normalHarness.calls(), ["ensure", "slash:hello", "provider:hello", "normal:hello"])
})

function createHarness(options: {
  promptText?: string | null
  pendingAttachmentCount?: number
  workflowScreen?: boolean
  instructionsEditorOpen?: boolean
  slashHandled?: boolean
  providerHandled?: boolean
  attached?: boolean
  submitWorkspaceShellCommand?: PromptSubmitCoordinatorDeps["submitWorkspaceShellCommand"]
} = {}) {
  const calls: string[] = []
  const footerMessages: Array<{ message: string; tone: "info" | "error" }> = []
  const slashCalls: Array<{
    rawPrompt: string
    allowSlashCommandSubmission: boolean
    trimmedPrompt: string
  }> = []
  let ensureCount = 0
  let clearCount = 0

  const coordinator = createPromptSubmitCoordinator({
    getPromptText: () => Object.prototype.hasOwnProperty.call(options, "promptText") ? options.promptText : "hello",
    ensureBackgroundPollersStarted: () => {
      ensureCount += 1
      calls.push("ensure")
    },
    getPendingAttachmentCount: () => options.pendingAttachmentCount ?? 0,
    clearPromptText: () => {
      clearCount += 1
    },
    workflowScreenShowing: () => options.workflowScreen ?? false,
    submitWorkspaceShellCommand: async (rawPrompt) => {
      calls.push(`workspace:${rawPrompt}`)
      if (options.submitWorkspaceShellCommand) {
        await options.submitWorkspaceShellCommand(rawPrompt)
      }
    },
    workflowNodeInstructionsEditorOpen: () => options.instructionsEditorOpen ?? false,
    submitSlashCommand: async (rawPrompt, submitOptions) => {
      slashCalls.push({ rawPrompt, ...submitOptions })
      calls.push(`slash:${rawPrompt}`)
      return options.slashHandled ?? false
    },
    submitProviderNamespacePrompt: async (rawPrompt) => {
      calls.push(`provider:${rawPrompt}`)
      return options.providerHandled ?? false
    },
    isAttached: () => options.attached ?? true,
    submitWorkflowPrompt: async (rawPrompt) => {
      calls.push(`workflow:${rawPrompt}`)
    },
    submitNormalPrompt: async (rawPrompt) => {
      calls.push(`normal:${rawPrompt}`)
    },
    flashFooter: (message, tone) => {
      footerMessages.push({ message, tone })
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
  })

  return {
    coordinator,
    calls: () => calls,
    footerMessages: () => footerMessages,
    slashCalls: () => slashCalls,
    ensureCount: () => ensureCount,
    clearCount: () => clearCount,
  }
}
