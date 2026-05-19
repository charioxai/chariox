import test from "node:test"
import assert from "node:assert/strict"

import {
  executeSlashCommand,
  parseSlashCommand,
  shouldClearCommandCenterForSlashCommand,
} from "./commands.js"

test("parseSlashCommand parses session commands with action and value", () => {
  assert.deepEqual(parseSlashCommand("/session attach abc123"), {
    kind: "session",
    raw: "/session attach abc123",
    action: "attach",
    args: ["abc123"],
    value: "abc123",
  })
})

test("parseSlashCommand preserves raw attachment command input", () => {
  assert.deepEqual(parseSlashCommand('/attach "foo bar.txt"'), {
    kind: "attachment",
    raw: '/attach "foo bar.txt"',
  })
})

test("parseSlashCommand parses workflow commands and args", () => {
  assert.deepEqual(parseSlashCommand("/workflow new review-flow"), {
    kind: "workflow",
    raw: "/workflow new review-flow",
    args: ["new", "review-flow"],
  })
})

test("parseSlashCommand parses workspace link commands and args", () => {
  assert.deepEqual(parseSlashCommand("/workspace link attach shared"), {
    kind: "workspace",
    raw: "/workspace link attach shared",
    args: ["link", "attach", "shared"],
  })
})

test("parseSlashCommand parses worktree commands and args", () => {
  assert.deepEqual(parseSlashCommand("/worktree create feature/web-cli"), {
    kind: "worktree",
    raw: "/worktree create feature/web-cli",
    args: ["create", "feature/web-cli"],
  })
})

test("parseSlashCommand parses cloud commands and args", () => {
  assert.deepEqual(parseSlashCommand("/cloud invite accept token-1"), {
    kind: "cloud",
    raw: "/cloud invite accept token-1",
    args: ["invite", "accept", "token-1"],
  })
})

test("shouldClearCommandCenterForSlashCommand only clears selector-backed commands", () => {
  assert.equal(shouldClearCommandCenterForSlashCommand(parseSlashCommand("/model openai/gpt-5")!), true)
  assert.equal(shouldClearCommandCenterForSlashCommand(parseSlashCommand("/session list")!), false)
  assert.equal(shouldClearCommandCenterForSlashCommand(parseSlashCommand("/stop")!), false)
})

test("executeSlashCommand dispatches to the matching handler", async () => {
  const calls: string[] = []
  const command = await executeSlashCommand("/view split", {
    onExit: () => calls.push("exit"),
    onWaiting: () => calls.push("waiting"),
    onStop: () => calls.push("stop"),
    onAttachment: () => calls.push("attachment"),
    onSession: () => calls.push("session"),
    onProvider: () => calls.push("provider"),
    onModel: () => calls.push("model"),
    onVariant: () => calls.push("variant"),
    onView: () => calls.push("view"),
    onAgent: () => calls.push("agent"),
    onKernel: () => calls.push("kernel"),
    onMachine: () => calls.push("machine"),
    onSlice: () => calls.push("slice"),
    onRelay: () => calls.push("relay"),
    onCloud: () => calls.push("cloud"),
    onConfig: () => calls.push("config"),
    onWorkspace: () => calls.push("workspace"),
    onWorktree: () => calls.push("worktree"),
    onWorkflow: () => calls.push("workflow"),
    onMcp: () => calls.push("mcp"),
    onSkill: () => calls.push("skill"),
    onEnv: () => calls.push("env"),
    onScript: () => calls.push("script"),
    onExtension: () => calls.push("extension"),
  })

  assert.deepEqual(calls, ["view"])
  assert.deepEqual(command, {
    kind: "view",
    raw: "/view split",
    value: "split",
  })
})

test("executeSlashCommand returns null for non-command input", async () => {
  const command = await executeSlashCommand("hello", {
    onExit: () => undefined,
    onWaiting: () => undefined,
    onStop: () => undefined,
    onAttachment: () => undefined,
    onSession: () => undefined,
    onProvider: () => undefined,
    onModel: () => undefined,
    onVariant: () => undefined,
    onView: () => undefined,
    onAgent: () => undefined,
    onKernel: () => undefined,
    onMachine: () => undefined,
    onSlice: () => undefined,
    onRelay: () => undefined,
    onCloud: () => undefined,
    onConfig: () => undefined,
    onWorkspace: () => undefined,
    onWorktree: () => undefined,
    onWorkflow: () => undefined,
    onMcp: () => undefined,
    onSkill: () => undefined,
    onEnv: () => undefined,
    onScript: () => undefined,
    onExtension: () => undefined,
  })

  assert.equal(command, null)
})

test("parseSlashCommand parses extension commands", () => {
  assert.deepEqual(parseSlashCommand("/mcp install browser"), {
    kind: "mcp",
    raw: "/mcp install browser",
    args: ["install", "browser"],
  })

  assert.deepEqual(parseSlashCommand("/skills list"), {
    kind: "skill",
    raw: "/skills list",
    args: ["list"],
  })

  assert.deepEqual(parseSlashCommand("/env register py"), {
    kind: "env",
    raw: "/env register py",
    args: ["register", "py"],
  })

  assert.deepEqual(parseSlashCommand("/script list"), {
    kind: "script",
    raw: "/script list",
    args: ["list"],
  })

  assert.deepEqual(parseSlashCommand("/extension grants script agent-1"), {
    kind: "extension",
    raw: "/extension grants script agent-1",
    args: ["grants", "script", "agent-1"],
  })
})
