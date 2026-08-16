import test from "node:test"
import assert from "node:assert/strict"

import {
  executeSlashCommand,
  parseSlashCommand,
  sharedShellCommandForSlashCommand,
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
  assert.deepEqual(parseSlashCommand("/loop Build a Kanban app"), {
    kind: "loop",
    raw: "/loop Build a Kanban app",
    prompt: "Build a Kanban app",
  })
  assert.deepEqual(parseSlashCommand("/goal Build a Kanban app"), {
    kind: "goal",
    raw: "/goal Build a Kanban app",
    prompt: "Build a Kanban app",
  })
  assert.deepEqual(parseSlashCommand("/loop"), {
    kind: "loop",
    raw: "/loop",
    prompt: "",
  })
  assert.deepEqual(parseSlashCommand("/goal"), {
    kind: "goal",
    raw: "/goal",
    prompt: "",
  })
})

test("parseSlashCommand parses the kernel notification center namespace", () => {
  assert.deepEqual(parseSlashCommand("/notifications connection remove connection-1 --confirm"), {
    kind: "notifications",
    raw: "/notifications connection remove connection-1 --confirm",
    args: ["connection", "remove", "connection-1", "--confirm"],
  })
  assert.equal(shouldClearCommandCenterForSlashCommand(
    parseSlashCommand("/notifications")!,
  ), true)
})

test("parseSlashCommand parses durable agent wait schedules", () => {
  assert.deepEqual(parseSlashCommand("/wait-in 0.05 Check once"), {
    kind: "wait",
    raw: "/wait-in 0.05 Check once",
    scheduleKind: "once",
    minutes: 0.05,
    prompt: "Check once",
    error: null,
  })
  assert.deepEqual(parseSlashCommand("/wait-every 5"), {
    kind: "wait",
    raw: "/wait-every 5",
    scheduleKind: "recurring",
    minutes: 5,
    prompt: "",
    error: null,
  })
  assert.deepEqual(parseSlashCommand("/wait-in later"), {
    kind: "wait",
    raw: "/wait-in later",
    scheduleKind: "once",
    minutes: null,
    prompt: "",
    error: "usage: /wait-in <minutes> [prompt]",
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

test("parseSlashCommand parses focused-agent mode and permission selections", () => {
  assert.deepEqual(parseSlashCommand("/mode plan"), {
    kind: "mode",
    raw: "/mode plan",
    value: "plan",
  })
  assert.deepEqual(parseSlashCommand("/permissions required"), {
    kind: "permissions",
    raw: "/permissions required",
    value: "required",
  })
})

test("parseSlashCommand parses collab commands and args", () => {
  assert.deepEqual(parseSlashCommand("/collab invite create"), {
    kind: "collab",
    raw: "/collab invite create",
    args: ["invite", "create"],
  })
})

test("sharedShellCommandForSlashCommand routes catalog-only kernel commands through the shared executor", () => {
  assert.equal(sharedShellCommandForSlashCommand("/collab invites"), "session invites")
  assert.equal(sharedShellCommandForSlashCommand("/workflow schedule preview --every 5m"), "workflow schedule preview --every 5m")
  assert.equal(sharedShellCommandForSlashCommand("/workflow code package export demo --out demo.json"), "workflow code package export demo --out demo.json")
  assert.equal(sharedShellCommandForSlashCommand("/workflow trigger show publication-1"), "workflow trigger show publication-1")
  assert.equal(sharedShellCommandForSlashCommand("/workflow registry list"), "workflow registry list")
  assert.equal(sharedShellCommandForSlashCommand("/workflow load demo"), "workflow load demo")
  assert.equal(
    sharedShellCommandForSlashCommand('/workflow run demo --endpoint entry --prompt "Run it"'),
    'workflow run demo --endpoint entry --prompt "Run it"',
  )
  assert.equal(
    sharedShellCommandForSlashCommand('/workflow run demo --prompt "Run it"'),
    'workflow run demo --prompt "Run it"',
  )
  assert.equal(sharedShellCommandForSlashCommand("/workflow run workflow-1 entry Run it"), null)
  assert.equal(sharedShellCommandForSlashCommand("/workflow schedule list"), null)
})

test("parseSlashCommand parses undo and fork commands with optional refs", () => {
  assert.deepEqual(parseSlashCommand("/undo"), {
    kind: "undo",
    raw: "/undo",
    args: [],
  })
  assert.deepEqual(parseSlashCommand("/undo agent-1"), {
    kind: "undo",
    raw: "/undo agent-1",
    args: ["agent-1"],
  })
  assert.deepEqual(parseSlashCommand("/fork qa"), {
    kind: "fork",
    raw: "/fork qa",
    args: ["qa"],
  })
  assert.deepEqual(parseSlashCommand("/agent fork qa"), {
    kind: "agent",
    raw: "/agent fork qa",
    args: ["fork", "qa"],
  })
})

test("shouldClearCommandCenterForSlashCommand only clears selector-backed commands", () => {
  assert.equal(shouldClearCommandCenterForSlashCommand(parseSlashCommand("/model openai/gpt-5")!), true)
  assert.equal(shouldClearCommandCenterForSlashCommand(parseSlashCommand("/undo")!), true)
  assert.equal(shouldClearCommandCenterForSlashCommand(parseSlashCommand("/fork agent-1")!), true)
  assert.equal(shouldClearCommandCenterForSlashCommand(parseSlashCommand("/loop Build a Kanban app")!), true)
  assert.equal(shouldClearCommandCenterForSlashCommand(parseSlashCommand("/goal Build a Kanban app")!), true)
  assert.equal(shouldClearCommandCenterForSlashCommand(parseSlashCommand("/wait-in 5 Check")!), true)
  assert.equal(shouldClearCommandCenterForSlashCommand(parseSlashCommand("/session list")!), false)
  assert.equal(shouldClearCommandCenterForSlashCommand(parseSlashCommand("/stop")!), false)
})

test("executeSlashCommand dispatches agent wait schedules", async () => {
  const calls: string[] = []
  const command = await executeSlashCommand("/wait-every 5 Check repeatedly", {
    onExit: () => undefined,
    onWaiting: () => undefined,
    onStop: () => undefined,
    onAttachment: () => undefined,
    onSession: () => undefined,
    onProvider: () => undefined,
    onModel: () => undefined,
    onVariant: () => undefined,
    onMode: () => undefined,
    onPermissions: () => undefined,
    onView: () => undefined,
    onUndo: () => undefined,
    onFork: () => undefined,
    onAgent: () => undefined,
    onKernel: () => undefined,
    onMachine: () => undefined,
    onSlice: () => undefined,
    onRelay: () => undefined,
    onCloud: () => undefined,
    onCollab: () => undefined,
    onConfig: () => undefined,
    onWorkspace: () => undefined,
    onWorktree: () => undefined,
    onWorkflow: () => undefined,
    onLoop: () => undefined,
    onGoal: () => undefined,
    onWait: (wait) => calls.push(`${wait.scheduleKind}:${wait.minutes}:${wait.prompt}`),
    onMcp: () => undefined,
    onSkill: () => undefined,
    onEnv: () => undefined,
    onScript: () => undefined,
    onCredential: () => undefined,
    onConnector: () => undefined,
    onExtension: () => undefined,
  })

  assert.deepEqual(calls, ["recurring:5:Check repeatedly"])
  assert.equal(command?.kind, "wait")
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
    onMode: () => calls.push("mode"),
    onPermissions: () => calls.push("permissions"),
    onView: () => calls.push("view"),
    onUndo: () => calls.push("undo"),
    onFork: () => calls.push("fork"),
    onAgent: () => calls.push("agent"),
    onKernel: () => calls.push("kernel"),
    onMachine: () => calls.push("machine"),
    onSlice: () => calls.push("slice"),
    onRelay: () => calls.push("relay"),
    onCloud: () => calls.push("cloud"),
    onCollab: () => calls.push("collab"),
    onConfig: () => calls.push("config"),
    onWorkspace: () => calls.push("workspace"),
    onWorktree: () => calls.push("worktree"),
    onWorkflow: () => calls.push("workflow"),
    onLoop: () => calls.push("loop"),
    onGoal: () => calls.push("goal"),
    onMcp: () => calls.push("mcp"),
    onSkill: () => calls.push("skill"),
    onEnv: () => calls.push("env"),
    onScript: () => calls.push("script"),
    onCredential: () => calls.push("credential"),
    onConnector: () => calls.push("connector"),
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
    onMode: () => undefined,
    onPermissions: () => undefined,
    onView: () => undefined,
    onUndo: () => undefined,
    onFork: () => undefined,
    onAgent: () => undefined,
    onKernel: () => undefined,
    onMachine: () => undefined,
    onSlice: () => undefined,
    onRelay: () => undefined,
    onCloud: () => undefined,
    onCollab: () => undefined,
    onConfig: () => undefined,
    onWorkspace: () => undefined,
    onWorktree: () => undefined,
    onWorkflow: () => undefined,
    onLoop: () => undefined,
    onGoal: () => undefined,
    onMcp: () => undefined,
    onSkill: () => undefined,
    onEnv: () => undefined,
    onScript: () => undefined,
    onCredential: () => undefined,
    onConnector: () => undefined,
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
