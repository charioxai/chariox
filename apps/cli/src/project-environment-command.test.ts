import assert from "node:assert/strict"
import test from "node:test"
import { handleEnvironmentSlashCommand } from "./capability-command-handlers.js"
import { projectEnvironmentCommand, type ProjectEnvironmentCommandDeps } from "./project-environment-command.js"

const view = {
  projectId: "p1", exists: true, revision: 3, setup: { script: "make" }, variables: "", changedAtMs: 1, machineLabel: "Builder", checkoutPath: "/w",
  setupState: { state: "notRun" },
  secrets: [{ name: "NOTION_TOKEN", presence: "missing" }, { name: "DB_URL", presence: "present" }],
  files: [{ path: ".env", presence: "present", sizeBytes: 12 }],
  mcps: [{ name: "github", source: { kind: "file", path: ".mcp.json" }, presence: "present" }, { name: "notion", source: { kind: "registration" }, presence: "missing" }],
  skills: [], proposals: [],
}
const operation = (extra: object = {}) => ({ operationId: "op-1", projectId: "p1", checkoutPath: "/w", kind: "setup", state: "running", startedAtMs: 1, endedAtMs: null, exitCode: null, reason: null, updates: 0, outputTail: "", entries: [], sourceLabel: null, ...extra })

function harness(kernels: readonly { id: string; label: string }[], reply: unknown) {
  const sent: any[] = []
  const deps: ProjectEnvironmentCommandDeps = {
    send: async (request) => { sent.push(request); return reply },
    room: () => ({ projectId: "p1", checkoutPath: "/w" }),
    otherKernels: async () => kernels,
    newOperationId: () => "op-1",
    now: () => 1000,
  }
  return { deps, sent }
}

test("status lists names and what is missing here, never a value", async () => {
  const h = harness([], { ProjectEnvironment: { environment: view } })
  const text = await projectEnvironmentCommand(h.deps, ["status"])
  assert.deepEqual(h.sent, [{ GetProjectEnvironment: { projectId: "p1", checkoutPath: "/w" } }])
  assert.match(text, /^Environment of Builder · revision 3 · Setup not run/)
  assert.match(text, /NOTION_TOKEN {2}Missing here/)
  assert.match(text, /notion {2}Chariox registration {2}Missing here/)
  assert.match(text, /github {2}\.mcp\.json {2}here/)
})

test("setup and check start their operations in the Room's Project", async () => {
  const setup = harness([], { EnvironmentOperation: { operation: operation() } })
  assert.equal(await projectEnvironmentCommand(setup.deps, ["setup"]), "Setup running (op-1)")
  assert.deepEqual(setup.sent, [{ RunProjectEnvironmentSetup: { projectId: "p1", operationId: "op-1", checkoutPath: "/w" } }])
  const check = harness([], { EnvironmentOperation: { operation: operation({ kind: "check" }) } })
  await projectEnvironmentCommand(check.deps, ["check"])
  assert.deepEqual(check.sent, [{ CheckProjectEnvironment: { projectId: "p1", operationId: "op-1", withAgent: false } }])
})

test("update takes the only other machine, a named one, and says which to name when there are several", async () => {
  const one = harness([{ id: "k-mac", label: "Miguel's Mac" }], { EnvironmentOperation: { operation: operation({ kind: "import" }) } })
  assert.equal(await projectEnvironmentCommand(one.deps, ["update"]), "Updating from Miguel's Mac (op-1)")
  assert.deepEqual(one.sent, [{ ImportProjectEnvironmentEntries: { projectId: "p1", operationId: "op-1", sourceKernelId: "k-mac", checkoutPath: "/w" } }])
  const two = harness([{ id: "k-mac", label: "Miguel's Mac" }, { id: "k-b", label: "Builder 2" }], { EnvironmentOperation: { operation: operation({ kind: "import" }) } })
  assert.equal(await projectEnvironmentCommand(two.deps, ["update"]), "Name the machine to update from: Miguel's Mac, Builder 2")
  assert.equal(two.sent.length, 0)
  await projectEnvironmentCommand(two.deps, ["update", "from", "builder", "2"])
  assert.equal(two.sent[0].ImportProjectEnvironmentEntries.sourceKernelId, "k-b")
  assert.equal(await projectEnvironmentCommand(harness([], null).deps, ["update"]), "No other machine of yours is online.")
})

test("/environment goes to the Project's Environment and /env keeps the Script runtimes", async () => {
  const notices: string[] = []
  const footers: string[] = []
  const calls: (readonly string[])[] = []
  const deps = {
    flashFooter: (message: string) => footers.push(message), appendNotice: (message: string) => notices.push(message),
    projectEnvironmentCommand: async (args: readonly string[]) => { calls.push(args); return "ok" },
    listEnvironments: async () => [],
  } as never
  await handleEnvironmentSlashCommand(deps, { kind: "env", raw: "/environment status", args: ["status"] })
  assert.deepEqual(calls, [["status"]]); assert.deepEqual(notices, ["ok"])
  await handleEnvironmentSlashCommand(deps, { kind: "env", raw: "/environment bogus", args: ["bogus"] })
  assert.match(footers.at(-1)!, /usage: \/environment status \| update \| setup \| check/)
  await handleEnvironmentSlashCommand(deps, { kind: "env", raw: "/env list", args: ["list"] })
  assert.equal(calls.length, 1); assert.match(notices.at(-1)!, /No Script runtimes registered/)
})
