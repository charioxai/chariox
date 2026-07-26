import assert from "node:assert/strict"
import test from "node:test"

import { createCommandCenterCommandExecutor } from "./command-center-command-executor.js"

test("command center command executor dispatches attachment commands with the raw input", async () => {
  const harness = createHarness()

  await harness.executor.execute("/attach ./notes.md")

  assert.deepEqual(harness.calls, ["attachment:/attach ./notes.md"])
  assert.deepEqual(harness.flashes, [])
})

test("command center command executor contains domain command failures in footer flashes", async () => {
  const harness = createHarness({
    onAgent: () => {
      throw new Error("agent unavailable")
    },
  })

  await harness.executor.execute("/agent focus missing")

  assert.deepEqual(harness.flashes, ["error:agent unavailable"])
})

test("command center command executor lets uncontained command failures propagate", async () => {
  const harness = createHarness({
    onSession: () => {
      throw new Error("session failed")
    },
  })

  await assert.rejects(() => harness.executor.execute("/session list"), /session failed/)
  assert.deepEqual(harness.flashes, [])
})

test("command center command executor dispatches wait commands", async () => {
  const harness = createHarness()

  await harness.executor.execute("/wait-in 3 Check later")

  assert.deepEqual(harness.calls, ["wait:once:3:Check later"])
})

function createHarness(overrides: Partial<Parameters<typeof createCommandCenterCommandExecutor>[0]> = {}) {
  const calls: string[] = []
  const flashes: string[] = []
  const deps: Parameters<typeof createCommandCenterCommandExecutor>[0] = {
    onExit: () => calls.push("exit"),
    onWaiting: () => calls.push("waiting"),
    onStop: () => calls.push("stop"),
    handleAttachmentCommand: (raw) => calls.push(`attachment:${raw}`),
    onSession: (command) => calls.push(`session:${command.action ?? ""}`),
    onProvider: (command) => calls.push(`provider:${command.value}`),
    onModel: (command) => calls.push(`model:${command.value}`),
    onVariant: (command) => calls.push(`variant:${command.value}`),
    onView: (command) => calls.push(`view:${command.value}`),
    onUndo: (command) => calls.push(`undo:${command.args.join(" ")}`),
    onFork: (command) => calls.push(`fork:${command.args.join(" ")}`),
    onAgent: (command) => calls.push(`agent:${command.args.join(" ")}`),
    onKernel: (command) => calls.push(`kernel:${command.args.join(" ")}`),
    onMachine: (command) => calls.push(`machine:${command.args.join(" ")}`),
    onSlice: (command) => calls.push(`slice:${command.args.join(" ")}`),
    onRelay: (command) => calls.push(`relay:${command.args.join(" ")}`),
    onCloud: (command) => calls.push(`cloud:${command.args.join(" ")}`),
    onCollab: (command) => calls.push(`collab:${command.args.join(" ")}`),
    onConfig: (command) => calls.push(`config:${command.args.join(" ")}`),
    onWorkspace: (command) => calls.push(`workspace:${command.args.join(" ")}`),
    onWorktree: (command) => calls.push(`worktree:${command.args.join(" ")}`),
    onWorkflow: (command) => calls.push(`workflow:${command.args.join(" ")}`),
    onLoop: (command) => calls.push(`loop:${command.prompt}`),
    onGoal: (command) => calls.push(`goal:${command.prompt}`),
    onWait: (command) => calls.push(`wait:${command.scheduleKind}:${command.minutes}:${command.prompt}`),
    onMcp: (command) => calls.push(`mcp:${command.args.join(" ")}`),
    onSkill: (command) => calls.push(`skill:${command.args.join(" ")}`),
    onEnv: (command) => calls.push(`env:${command.args.join(" ")}`),
    onScript: (command) => calls.push(`script:${command.args.join(" ")}`),
    onCredential: (command) => calls.push(`credential:${command.args.join(" ")}`),
    onConnector: (command) => calls.push(`connector:${command.args.join(" ")}`),
    onExtension: (command) => calls.push(`extension:${command.args.join(" ")}`),
    flashFooter: (message, tone) => flashes.push(`${tone}:${message}`),
    formatError: (error) => error instanceof Error ? error.message : String(error),
    ...overrides,
  }
  return {
    calls,
    flashes,
    executor: createCommandCenterCommandExecutor(deps),
  }
}
