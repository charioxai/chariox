import assert from "node:assert/strict"
import test from "node:test"

import type { EnvironmentOperationView, ProjectEnvironmentView } from "@chariox/kernel-client/kernel-types"
import { createProjectEnvironmentController } from "./project-environment-controller.js"
import { setupStateText } from "./project-environment-projection.js"

const NOW = 1_700_000_000_000
const HOUR = 3_600_000

function environment(patch: Partial<ProjectEnvironmentView> = {}): ProjectEnvironmentView {
  return {
    projectId: "project-1",
    exists: true,
    revision: 2,
    setup: { script: "make setup" },
    variables: "PORT=3000",
    secrets: [{ name: "API_KEY", presence: "present" }, { name: "DB_URL", presence: "missing" }],
    files: [{ path: ".env.local", presence: "missing", sizeBytes: null }, { path: "certs/dev.pem", presence: "present", sizeBytes: 2048 }],
    mcps: [], skills: [],
    changedAtMs: NOW,
    machineLabel: "Miguel's Mac",
    checkoutPath: "/work/project",
    setupState: { state: "notRun" },
    proposals: [],
    ...patch,
  }
}

function operation(patch: Partial<EnvironmentOperationView> = {}): EnvironmentOperationView {
  return {
    operationId: "op-1", projectId: "project-1", checkoutPath: "/work/project", kind: "setup", state: "running",
    startedAtMs: NOW, endedAtMs: null, exitCode: null, reason: null, updates: 0, outputTail: "installing\n", entries: [], sourceLabel: null, ...patch,
  }
}

type Reply = unknown | Error | (() => unknown)

function harness(initial: ProjectEnvironmentView = environment(), kernels: readonly { id: string; label: string }[] = []) {
  const requests: Record<string, any>[] = []
  const replies: Reply[] = []
  // Reads the kernel holds until the operation changes: they answer when the test calls `release`.
  const held: Array<(value: unknown) => void> = []
  const scheduled: Array<{ callback: () => void; delayMs: number }> = []
  let renders = 0
  let operationIds = 0
  const controller = createProjectEnvironmentController({
    otherKernels: async () => kernels,
    send: async (request) => {
      requests.push(request as Record<string, any>)
      if ((request as any).GetEnvironmentOperation?.waitAfterUpdates !== undefined) return new Promise((resolve) => held.push(resolve))
      const next = replies.shift()
      if (next === undefined) throw new Error(`unexpected request ${Object.keys(request as object)[0]}`)
      const value = typeof next === "function" ? (next as () => unknown)() : next
      if (value instanceof Error) throw value
      return value
    },
    render: () => { renders++ },
    schedule: (callback, delayMs) => { scheduled.push({ callback, delayMs }) },
    newOperationId: () => `op-${++operationIds}`,
    now: () => NOW,
    pageSize: () => 60,
    textWidth: () => 80,
  })
  const settle = async () => {
    for (let i = 0; i < 20; i++) await Promise.resolve()
    await new Promise((resolve) => setImmediate(resolve))
  }
  const key = (name: string, extra: Record<string, unknown> = {}) => controller.handleKey({ name, ...extra })
  const type = (text: string) => { for (const char of text) key(char, { sequence: char }) }
  const text = () => controller.view().rows.map((row) => row.text).join("\n")
  const open = async (...then: Reply[]) => {
    replies.push({ ProjectEnvironment: { environment: initial } }, ...then)
    await controller.open("project-1")
  }
  const focusOn = (target: string) => {
    for (let i = 0; i < 30; i++) key("up")
    for (let i = 0; i < 20 && !controller.view().rows.some((row) => row.focused && row.text.includes(target)); i++) key("down")
  }
  const release = async (value: unknown) => { held.shift()!(value); await settle() }
  return { controller, requests, replies, scheduled, key, type, text, open, settle, focusOn, release, held, renders: () => renders }
}

test("the dialog shows one machine line and the four sections from the kernel", async () => {
  const h = harness()
  await h.open()
  assert.deepEqual(h.requests, [{ GetProjectEnvironment: { projectId: "project-1" } }])
  const text = h.text()
  assert.match(text, /^Miguel's Mac · Setup not run\s+Run setup/)
  for (const part of ["Setup\n  make setup", "Variables\n  PORT=3000", "API_KEY", "In Vault", "DB_URL", "Missing here", "+ Add secret", "Files outside Git", ".env.local", "certs/dev.pem", "Present · 2.0 KB", "+ Add file"]) {
    assert(text.includes(part), `missing ${part} in\n${text}`)
  }
})

test("setup state text covers every state", () => {
  assert.equal(setupStateText({ state: "notRun" }, NOW), "Setup not run")
  assert.equal(setupStateText({ state: "ok", operationId: "op-1", endedAtMs: NOW - 3 * HOUR }, NOW), "Setup ran 3 h ago")
  assert.equal(setupStateText({ state: "failed", operationId: "op-1", endedAtMs: NOW - 2 * HOUR, exitCode: 1, reason: null }, NOW), "Setup failed 2 h ago")
  assert.equal(setupStateText({ state: "cancelled", operationId: "op-1", endedAtMs: NOW - 90_000 }, NOW), "Setup cancelled 1 m ago")
  assert.equal(setupStateText({ state: "running", operationId: "o", startedAtMs: NOW }, NOW), "Running")
})

test("Vault locked secrets say so", async () => {
  const h = harness(environment({ secrets: [{ name: "API_KEY", presence: "locked" }] }))
  await h.open()
  assert.match(h.text(), /API_KEY\s+Vault locked/)
})

test("a failed load shows the reason in the dialog and closes on Esc", async () => {
  const h = harness()
  h.replies.push(new Error("local transport `project.get` failed: project `x` was not found"))
  await h.controller.open("project-1")
  assert.equal(h.controller.isOpen(), true)
  assert.match(h.controller.view().message ?? "", /was not found/)
  h.key("escape")
  assert.equal(h.controller.isOpen(), false)
})

test("editing the script saves with the loaded revision when Esc leaves the field", async () => {
  const h = harness()
  await h.open()
  h.key("down")
  h.key("return")
  h.type(" && make test")
  assert.match(h.text(), /Setup\s+Unsaved\n  make setup && make test▏/)
  h.replies.push({ ProjectEnvironmentSaved: { environment: environment({ revision: 3, setup: { script: "make setup && make test" } }) } })
  h.controller.dismiss()
  await h.settle()
  assert.deepEqual(h.requests.at(-1), { SaveProjectEnvironment: {
    projectId: "project-1", expectedRevision: 2, setup: { script: "make setup && make test" }, variables: "PORT=3000",
    secrets: [{ name: "API_KEY" }, { name: "DB_URL" }], files: [{ path: ".env.local" }, { path: "certs/dev.pem" }], mcps: [], skills: [],
  } })
  assert(!h.text().includes("Unsaved"))
  assert(h.text().includes("make setup && make test"))
})

test("leaving a field without a change sends nothing", async () => {
  const h = harness()
  await h.open()
  h.key("down")
  h.key("return")
  h.controller.dismiss()
  await h.settle()
  assert.equal(h.requests.length, 1)
})

test("an edit completed during a pending save is saved at the next revision", async () => {
  const h = harness()
  await h.open()
  let finishSave!: (value: unknown) => void
  h.replies.push(() => new Promise((resolve) => { finishSave = resolve }))
  h.focusOn("make setup")
  h.key("return")
  h.type(" && make test")
  h.controller.dismiss()
  await h.settle()
  h.focusOn("PORT=3000")
  h.key("return")
  h.type("&DEBUG=1")
  h.controller.dismiss()
  await h.settle()
  h.replies.push({ ProjectEnvironmentSaved: { environment: environment({ revision: 4, setup: { script: "make setup && make test" }, variables: "PORT=3000&DEBUG=1" }) } })
  finishSave({ ProjectEnvironmentSaved: { environment: environment({ revision: 3, setup: { script: "make setup && make test" } }) } })
  await h.settle()
  const saves = h.requests.filter((request) => request.SaveProjectEnvironment).map((request) => request.SaveProjectEnvironment)
  assert.equal(saves.length, 2)
  assert.equal(saves[1].expectedRevision, 3)
  assert.equal(saves[1].setup.script, "make setup && make test")
  assert.equal(saves[1].variables, "PORT=3000&DEBUG=1")
  assert(h.text().includes("PORT=3000&DEBUG=1"))
  assert(!h.text().includes("Unsaved"))
})

test("a stale revision reloads the record and keeps the typed text for another save", async () => {
  const h = harness()
  await h.open()
  h.focusOn("PORT=3000")
  h.key("return")
  h.type("&DEBUG=1")
  h.replies.push({ ProjectEnvironmentConflict: { environment: environment({ revision: 5, setup: { script: "make other" }, variables: "PORT=4000" }) } })
  h.controller.dismiss()
  await h.settle()
  assert.equal(h.controller.view().message, "Changed elsewhere")
  const text = h.text()
  assert(text.includes("PORT=3000&DEBUG=1"), text)
  assert(text.includes("make other"), "untouched fields show the reloaded record")
  h.replies.push({ ProjectEnvironmentSaved: { environment: environment({ revision: 6, variables: "PORT=3000&DEBUG=1" }) } })
  h.key("down")
  h.key("up")
  h.key("return")
  h.controller.dismiss()
  await h.settle()
  assert.equal(h.requests.at(-1)!.SaveProjectEnvironment.expectedRevision, 5)
  assert.equal(h.requests.at(-1)!.SaveProjectEnvironment.variables, "PORT=3000&DEBUG=1")
})

test("adding a secret saves the name, then sends the masked value to the Vault without echoing it", async () => {
  const h = harness()
  await h.open()
  h.focusOn("+ Add secret")
  h.key("return")
  h.type("NEW_TOKEN")
  assert.match(h.text(), /Name NEW_TOKEN▏/)
  const withToken = environment({ revision: 3, secrets: [...environment().secrets, { name: "NEW_TOKEN", presence: "missing" }] })
  h.replies.push({ ProjectEnvironmentSaved: { environment: withToken } })
  h.key("return")
  await h.settle()
  assert.deepEqual(h.requests.at(-1)!.SaveProjectEnvironment.secrets, [{ name: "API_KEY" }, { name: "DB_URL" }, { name: "NEW_TOKEN" }])
  assert.equal(h.requests.at(-1)!.SaveProjectEnvironment.expectedRevision, 2)
  h.type("hunter2-secret")
  const masked = h.text()
  assert(!masked.includes("hunter2"), "value must never reach the rows")
  assert.match(masked, /NEW_TOKEN\s+Value •{14}▏/)
  h.replies.push({ ProjectEnvironmentSaved: { environment: environment({ revision: 3, secrets: [...environment().secrets, { name: "NEW_TOKEN", presence: "present" }] }) } })
  h.key("return")
  await h.settle()
  assert.deepEqual(h.requests.at(-1), { SetProjectEnvironmentSecret: { projectId: "project-1", name: "NEW_TOKEN", value: "hunter2-secret" } })
  assert.match(h.text(), /NEW_TOKEN\s+In Vault/)
  assert(!JSON.stringify(h.controller.view()).includes("hunter2"))
})

test("a Vault failure shows in the dialog and drops the typed value", async () => {
  const h = harness()
  await h.open()
  h.focusOn("DB_URL")
  h.key("return")
  h.type("secret-value")
  h.replies.push(new Error("Vault is locked"))
  h.key("return")
  await h.settle()
  assert.equal(h.controller.view().message, "Vault is locked")
  assert(!JSON.stringify(h.controller.view()).includes("secret-value"))
  assert.match(h.text(), /DB_URL\s+Missing here/)
})

test("Esc cancels a secret value without sending it", async () => {
  const h = harness()
  await h.open()
  h.focusOn("DB_URL")
  h.key("return")
  h.type("abc")
  h.controller.dismiss()
  await h.settle()
  assert.equal(h.requests.length, 1)
  assert.equal(h.controller.isOpen(), true)
})

test("removing a secret name and a file path saves the shorter lists", async () => {
  const h = harness()
  await h.open()
  h.focusOn("DB_URL")
  h.replies.push({ ProjectEnvironmentSaved: { environment: environment({ revision: 3, secrets: [{ name: "API_KEY", presence: "present" }] }) } })
  h.key("x")
  await h.settle()
  assert.deepEqual(h.requests.at(-1)!.SaveProjectEnvironment.secrets, [{ name: "API_KEY" }])
  h.focusOn(".env.local")
  h.replies.push({ ProjectEnvironmentSaved: { environment: environment({ revision: 4, files: [{ path: "certs/dev.pem", presence: "present", sizeBytes: 2048 }] }) } })
  h.key("delete")
  await h.settle()
  assert.deepEqual(h.requests.at(-1)!.SaveProjectEnvironment.files, [{ path: "certs/dev.pem" }])
  assert.equal(h.requests.at(-1)!.SaveProjectEnvironment.expectedRevision, 3)
})

test("adding a file path saves it", async () => {
  const h = harness()
  await h.open()
  h.focusOn("+ Add file")
  h.key("return")
  h.type("config/local.json")
  h.replies.push({ ProjectEnvironmentSaved: { environment: environment({ revision: 3 }) } })
  h.key("return")
  await h.settle()
  assert.deepEqual(h.requests.at(-1)!.SaveProjectEnvironment.files.at(-1), { path: "config/local.json" })
})

test("Run setup starts an operation, follows it with held reads and shows the output tail", async () => {
  const h = harness()
  await h.open()
  h.replies.push({ EnvironmentOperation: { operation: operation() } })
  h.replies.push({ ProjectEnvironment: { environment: environment({ setupState: { state: "running", operationId: "op-1", startedAtMs: NOW } }) } })
  h.key("return")
  await h.settle()
  assert.deepEqual(h.requests[1], { RunProjectEnvironmentSetup: { projectId: "project-1", operationId: "op-1" } })
  assert.match(h.text(), /Miguel's Mac · Running\s+Stop/)
  assert(h.text().includes("  installing"))
  assert.deepEqual(h.requests.find((request) => request.GetEnvironmentOperation), { GetEnvironmentOperation: { operationId: "op-1", waitAfterUpdates: 0 } }, "the run is followed by a held read")
  assert.deepEqual(h.scheduled, [], "nothing is polled on a timer")

  await h.release({ EnvironmentOperation: { operation: operation({ updates: 1, outputTail: "installing\ncompiling\n" }) } })
  assert(h.text().includes("  compiling"))
  assert.deepEqual(h.requests.at(-1), { GetEnvironmentOperation: { operationId: "op-1", waitAfterUpdates: 1 } })

  h.replies.push({ ProjectEnvironment: { environment: environment({ setupState: { state: "ok", operationId: "op-1", endedAtMs: NOW + 5 } }) } })
  await h.release({ EnvironmentOperation: { operation: operation({ state: "succeeded", updates: 2, endedAtMs: NOW + 5, exitCode: 0, outputTail: "installing\ncompiling\ndone\n" }) } })
  assert.deepEqual(h.requests.at(-1), { GetProjectEnvironment: { projectId: "project-1" } })
  assert.match(h.text(), /Setup ran just now\s+Run setup/)
  assert.equal(h.held.length, 0)
})

test("Stop cancels the running operation and the record shows cancelled", async () => {
  const running = environment({ setupState: { state: "running", operationId: "op-9", startedAtMs: NOW } })
  const h = harness(running)
  await h.open({ EnvironmentOperation: { operation: operation({ operationId: "op-9" }) } })
  await h.settle()
  assert.deepEqual(h.requests.at(-1), { GetEnvironmentOperation: { operationId: "op-9", waitAfterUpdates: 0 } })
  h.replies.push({ EnvironmentOperation: { operation: operation({ operationId: "op-9", state: "cancelled", endedAtMs: NOW + 1 }) } })
  h.replies.push({ ProjectEnvironment: { environment: environment({ setupState: { state: "cancelled", operationId: "op-9", endedAtMs: NOW + 1 } }) } })
  h.key("return")
  await h.settle()
  assert.deepEqual(h.requests.find((request) => request.CancelEnvironmentOperation), { CancelEnvironmentOperation: { operationId: "op-9" } })
  assert.match(h.text(), /Setup cancelled just now\s+Run setup/)
  const sent = h.requests.length
  await h.release({ EnvironmentOperation: { operation: operation({ operationId: "op-9", state: "cancelled", updates: 1 }) } })
  assert.equal(h.requests.length, sent, "a stopped follow sends nothing more")
})

test("a failed run shows the reason under the script", async () => {
  const h = harness()
  await h.open()
  h.replies.push({ EnvironmentOperation: { operation: operation({ state: "failed", operationId: "op-1", endedAtMs: NOW, exitCode: 1, reason: "Vault is locked", outputTail: "" }) } })
  h.replies.push({ ProjectEnvironment: { environment: environment({ setupState: { state: "failed", operationId: "op-1", endedAtMs: NOW, exitCode: 1, reason: "Vault is locked" } }) } })
  h.key("return")
  await h.settle()
  assert.match(h.text(), /Setup failed just now/)
  const reason = h.controller.view().rows.find((row) => row.text.includes("Vault is locked"))
  assert.equal(reason?.tone, "error")
})

test("Run setup first saves unsaved text and does not run when the save conflicts", async () => {
  const h = harness()
  await h.open()
  h.key("down")
  h.key("return")
  h.type("!")
  h.controller.dismiss()
  h.replies.push({ ProjectEnvironmentConflict: { environment: environment({ revision: 4 }) } })
  await h.settle()
  h.key("up")
  h.key("return")
  await h.settle()
  assert(h.requests.every((request) => !("RunProjectEnvironmentSetup" in request)))
})

test("a response that arrives after the dialog closed is ignored", async () => {
  const h = harness()
  let release: (value: unknown) => void = () => undefined
  h.replies.push(() => new Promise((resolve) => { release = resolve }))
  const opening = h.controller.open("project-1")
  h.controller.close()
  release({ ProjectEnvironment: { environment: environment() } })
  await opening
  assert.equal(h.controller.isOpen(), false)
  assert.match(h.text(), /Loading Environment…|Environment unavailable/)
})

test("Ctrl+C and Ctrl+E pass through and keys in the dialog do not leak to the prompt", async () => {
  const h = harness()
  await h.open()
  assert.equal(h.key("c", { ctrl: true }), false)
  assert.equal(h.key("e", { ctrl: true }), false)
  assert.equal(h.key("q"), true)
  assert.equal(h.key("down", { eventType: "release" }), true)
})

test("a paste lands in the active field and is refused when no field is active", async () => {
  const h = harness()
  await h.open()
  assert.equal(h.controller.handlePaste("ignored"), false)
  h.key("down")
  h.key("return")
  assert.equal(h.controller.handlePaste("\nline two\r\nline three"), true)
  assert(h.text().includes("line three"))
})

test("a reopened dialog shows the output of the last finished run once", async () => {
  const finished = environment({ setupState: { state: "failed", operationId: "op-3", endedAtMs: NOW - 2 * HOUR, exitCode: 1, reason: null } })
  const h = harness(finished)
  await h.open({ EnvironmentOperation: { operation: operation({ operationId: "op-3", state: "failed", endedAtMs: NOW - 2 * HOUR, exitCode: 1, outputTail: "pnpm install failed\n" }) } })
  assert.deepEqual(h.requests.map((request) => Object.keys(request)[0]), ["GetProjectEnvironment", "GetEnvironmentOperation"])
  assert.match(h.text(), /pnpm install failed/)
  assert.equal(h.scheduled.length, 0, "a finished run is not polled")
})

const MAC = [{ id: "kernel-mac", label: "Miguel's Mac" }]
const importOperation = (patch: Partial<EnvironmentOperationView> = {}) => operation({ operationId: "op-1", kind: "import", sourceLabel: "Miguel's Mac", outputTail: "", ...patch })

test("Update and Import here are offered only when the user has another kernel", async () => {
  const alone = harness()
  await alone.open()
  assert.doesNotMatch(alone.text(), /Update/)
  alone.focusOn(".env.local")
  assert.doesNotMatch(alone.controller.view().hint, /import/i)
  alone.key("i")
  await alone.settle()
  assert.equal(alone.requests.length, 1, "no import without a source")

  const two = harness(environment(), MAC)
  await two.open()
  await two.settle()
  assert.match(two.text(), /Update from Miguel's Mac\s+Update/)
  two.focusOn(".env.local")
  assert.match(two.controller.view().hint, /I import here/)
  two.focusOn("API_KEY")
  assert.doesNotMatch(two.controller.view().hint, /import here/i, "a row this machine has is not imported")
})

test("Import here sends the focused missing entry to the chosen kernel and the row follows it from importing to imported", async () => {
  const h = harness(environment(), MAC)
  await h.open()
  await h.settle()
  h.focusOn("DB_URL")
  const importing = importOperation({ entries: [{ kind: "secret", name: "DB_URL", state: "importing", reason: null }] })
  h.replies.push({ EnvironmentOperation: { operation: importing } })
  h.key("i")
  await h.settle()
  assert.deepEqual(h.requests.find((request) => request.ImportProjectEnvironmentEntries), { ImportProjectEnvironmentEntries: { projectId: "project-1", operationId: "op-1", sourceKernelId: "kernel-mac", entries: [{ kind: "secret", name: "DB_URL" }] } })
  assert.match(h.text(), /DB_URL\s+Importing from Miguel's Mac/)
  assert.match(h.text(), /Updating from Miguel's Mac · 0 of 1\s+Stop/)
  assert.deepEqual(h.requests.at(-1), { GetEnvironmentOperation: { operationId: "op-1", waitAfterUpdates: 0 } })
  h.key("i")
  await h.settle()
  assert.equal(h.requests.filter((request) => request.ImportProjectEnvironmentEntries).length, 1, "no second import while one runs")

  h.replies.push({ ProjectEnvironment: { environment: environment({ secrets: [{ name: "API_KEY", presence: "present" }, { name: "DB_URL", presence: "present" }] }) } })
  await h.release({ EnvironmentOperation: { operation: importOperation({ state: "succeeded", updates: 3, endedAtMs: NOW + 1, entries: [{ kind: "secret", name: "DB_URL", state: "imported", reason: null }] }) } })
  assert.match(h.text(), /DB_URL\s+Imported from Miguel's Mac/)
  assert.match(h.text(), /Update from Miguel's Mac\s+Update/)
  assert.equal(h.controller.view().rows.find((row) => row.text.includes("DB_URL"))?.tone, "text")
})

test("A registered MCP this machine lacks reads Missing here, I imports it from the chosen kernel and the row follows", async () => {
  const h = harness(environment({
    mcps: [{ name: "github", source: { kind: "file", path: ".mcp.json" }, presence: "present" }, { name: "notion", source: { kind: "registration" }, presence: "missing" }],
    skills: [{ name: "dataviz", source: { kind: "registration" }, presence: "present" }],
  }), MAC)
  await h.open()
  await h.settle()
  assert.match(h.text(), /notion\s+Missing here/)
  assert.match(h.text(), /dataviz\s+Registered/)
  h.focusOn("github")
  h.key("i")
  await h.settle()
  assert.equal(h.requests.filter((request) => request.ImportProjectEnvironmentEntries).length, 0, "a project file is Git's: nothing to import")
  h.focusOn("notion")
  assert.match(h.controller.view().hint, /I import here/)
  h.replies.push({ EnvironmentOperation: { operation: importOperation({ entries: [{ kind: "mcp", name: "notion", state: "importing", reason: null }] }) } })
  h.key("i")
  await h.settle()
  assert.deepEqual(h.requests.find((request) => request.ImportProjectEnvironmentEntries), { ImportProjectEnvironmentEntries: { projectId: "project-1", operationId: "op-1", sourceKernelId: "kernel-mac", entries: [{ kind: "mcp", name: "notion" }] } })
  assert.match(h.text(), /notion\s+Importing from Miguel's Mac/)
})

test("Update brings every missing entry, shows n of m, and a row that failed says why in the warning tone", async () => {
  const h = harness(environment(), MAC)
  await h.open()
  await h.settle()
  h.focusOn("Update from")
  const running = importOperation({ entries: [
    { kind: "secret", name: "DB_URL", state: "imported", reason: null },
    { kind: "file", name: ".env.local", state: "pending", reason: null },
  ] })
  h.replies.push({ EnvironmentOperation: { operation: running } })
  h.key("return")
  await h.settle()
  assert.deepEqual(h.requests.find((request) => request.ImportProjectEnvironmentEntries), { ImportProjectEnvironmentEntries: { projectId: "project-1", operationId: "op-1", sourceKernelId: "kernel-mac" } })
  assert.match(h.text(), /Updating from Miguel's Mac · 1 of 2\s+Stop/)
  assert.match(h.text(), /Run setup/)

  h.replies.push({ ProjectEnvironment: { environment: environment() } })
  await h.release({ EnvironmentOperation: { operation: importOperation({ state: "succeeded", updates: 4, endedAtMs: NOW + 2, entries: [
    { kind: "secret", name: "DB_URL", state: "imported", reason: null },
    { kind: "file", name: ".env.local", state: "failed", reason: "Not on Miguel's Mac" },
  ] }) } })
  assert.match(h.text(), /\.env\.local\s+Not on Miguel's Mac/)
  assert.equal(h.controller.view().rows.find((row) => row.text.includes(".env.local"))?.tone, "warning")
})

test("Stop ends a running Update and keeps what landed; an Update that reached nothing says why on its row", async () => {
  const h = harness(environment(), MAC)
  await h.open()
  await h.settle()
  h.focusOn("Update from")
  h.replies.push({ EnvironmentOperation: { operation: importOperation({ entries: [{ kind: "secret", name: "DB_URL", state: "pending", reason: null }] }) } })
  h.key("return")
  await h.settle()
  h.replies.push({ EnvironmentOperation: { operation: importOperation({ state: "cancelled", updates: 2, endedAtMs: NOW + 1, entries: [{ kind: "secret", name: "DB_URL", state: "pending", reason: null }] }) } })
  h.replies.push({ ProjectEnvironment: { environment: environment() } })
  h.key("return")
  await h.settle()
  assert.deepEqual(h.requests.find((request) => request.CancelEnvironmentOperation), { CancelEnvironmentOperation: { operationId: "op-1" } })
  assert.match(h.text(), /Update from Miguel's Mac\s+Update/)

  h.replies.push({ EnvironmentOperation: { operation: importOperation({ operationId: "op-2", state: "failed", reason: "Source offline", endedAtMs: NOW + 3 }) } })
  h.key("return")
  await h.settle()
  assert.match(h.text(), /Source offline\s+Update/)
  assert.equal(h.controller.view().rows.find((row) => row.text.includes("Source offline"))?.tone, "warning")
})

test("with two other kernels M chooses the source", async () => {
  const h = harness(environment(), [...MAC, { id: "kernel-b3", label: "b3 builder" }])
  await h.open()
  await h.settle()
  assert.match(h.text(), /Update from Miguel's Mac/)
  assert.match(h.controller.view().hint, /M machine/)
  h.key("m")
  assert.match(h.text(), /Update from b3 builder/)
})

test("an Environment that does not exist yet offers Draft with agent; it runs on the kernel and the record follows", async () => {
  const h = harness(environment({ exists: false, revision: 0, setup: { script: "" }, variables: "", secrets: [], files: [] }))
  await h.open()
  assert.match(h.text(), /No Environment for this Project yet\./)
  h.focusOn("Draft with agent")
  h.replies.push({ EnvironmentOperation: { operation: operation({ operationId: "op-1", kind: "draft", outputTail: "" }) } })
  h.key("return")
  await h.settle()
  assert.deepEqual(h.requests.find((request) => request.DraftProjectEnvironment), { DraftProjectEnvironment: { projectId: "project-1", operationId: "op-1" } })
  assert.match(h.text(), /Drafting with agent\s+Stop/)
  h.replies.push({ ProjectEnvironment: { environment: environment() } })
  await h.release({ EnvironmentOperation: { operation: operation({ operationId: "op-1", kind: "draft", state: "succeeded", updates: 2, endedAtMs: NOW + 1, outputTail: "" }) } })
  assert.match(h.text(), /Check if up to date with agent/)
  assert.doesNotMatch(h.text(), /No Environment/)
})

test("a draft that fails says why under the button, which stays", async () => {
  const h = harness(environment({ exists: false, revision: 0, setup: { script: "" }, variables: "", secrets: [], files: [] }))
  await h.open()
  h.focusOn("Draft with agent")
  h.replies.push({ EnvironmentOperation: { operation: operation({ operationId: "op-1", kind: "draft", state: "failed", reason: "The agent did not answer with an Environment", endedAtMs: NOW, outputTail: "" }) } })
  h.key("return")
  await h.settle()
  assert.match(h.text(), /The agent did not answer with an Environment/)
  assert.match(h.text(), /Draft with agent/)
})

test("Check with agent leaves proposals; Enter accepts one as a Save and X dismisses one", async () => {
  const proposals = [
    { id: "a", field: "setup", change: "add", text: "pnpm prisma generate", evidence: { path: "AGENTS.md", excerpt: "run prisma generate" } },
    { id: "b", field: "secret", change: "add", text: "STRIPE_SECRET_KEY", evidence: { path: ".env.example", excerpt: "STRIPE_SECRET_KEY=" } },
  ] as const
  const h = harness()
  await h.open()
  h.focusOn("Check if up to date")
  h.replies.push({ EnvironmentOperation: { operation: operation({ operationId: "op-1", kind: "check", outputTail: "" }) } })
  h.key("return")
  await h.settle()
  assert.deepEqual(h.requests.find((request) => request.CheckProjectEnvironment), { CheckProjectEnvironment: { projectId: "project-1", operationId: "op-1", withAgent: true } })
  assert.match(h.text(), /Checking with agent\s+Stop/)
  h.replies.push({ ProjectEnvironment: { environment: environment({ proposals }) } })
  await h.release({ EnvironmentOperation: { operation: operation({ operationId: "op-1", kind: "check", state: "succeeded", updates: 2, endedAtMs: NOW + 1, outputTail: "" }) } })
  assert.match(h.text(), /\+ pnpm prisma generate\s+Proposed · AGENTS\.md/)
  assert.match(h.text(), /\+ STRIPE_SECRET_KEY\s+Proposed · \.env\.example/)

  h.focusOn("pnpm prisma generate")
  h.replies.push({ ProjectEnvironmentSaved: { environment: environment({ revision: 3, setup: { script: "make setup\npnpm prisma generate" }, proposals: [proposals[1]] }) } })
  h.key("return")
  await h.settle()
  assert.deepEqual(h.requests.find((request) => request.ResolveEnvironmentProposal), { ResolveEnvironmentProposal: { projectId: "project-1", proposalId: "a", accept: true } })
  assert.doesNotMatch(h.text(), /\+ pnpm prisma generate\s+Proposed/)

  h.focusOn("STRIPE_SECRET_KEY")
  h.replies.push({ ProjectEnvironmentSaved: { environment: environment({ revision: 3, proposals: [] }) } })
  h.key("x")
  await h.settle()
  assert.deepEqual(h.requests.filter((request) => request.ResolveEnvironmentProposal).at(-1), { ResolveEnvironmentProposal: { projectId: "project-1", proposalId: "b", accept: false } })
  assert.doesNotMatch(h.text(), /Proposed/)
})

test("MCPs and Skills that project files declare are listed; a missing declaration is a warning and X removes the row", async () => {
  const h = harness(environment({
    mcps: [{ name: "github", source: { kind: "file", path: ".mcp.json" }, presence: "present" }, { name: "notion", source: { kind: "file", path: ".mcp.json" }, presence: "missing" }],
    skills: [{ name: "dataviz", source: { kind: "file", path: ".claude/skills/dataviz/SKILL.md" }, presence: "present" }],
  }))
  await h.open()
  assert.match(h.text(), /MCPs\n {2}github\s+In project files\n {2}notion\s+Missing here\nSkills\n {2}dataviz\s+In project files/)
  assert.equal(h.controller.view().rows.find((row) => row.text.includes("notion"))?.tone, "warning")
  h.focusOn("notion")
  h.replies.push({ ProjectEnvironmentSaved: { environment: environment({ revision: 3, mcps: [{ name: "github", source: { kind: "file", path: ".mcp.json" }, presence: "present" }] }) } })
  h.key("x")
  await h.settle()
  const save = h.requests.find((request) => request.SaveProjectEnvironment)!.SaveProjectEnvironment
  assert.deepEqual(save.mcps, [{ name: "github", source: { kind: "file", path: ".mcp.json" } }])
  assert.deepEqual(save.skills, [{ name: "dataviz", source: { kind: "file", path: ".claude/skills/dataviz/SKILL.md" } }], "the skills are saved as they were")
})

test("C checks the Project against its own files without an agent, and proposals for an MCP read like the others", async () => {
  const h = harness()
  await h.open()
  assert.match(h.controller.view().hint, /C check/)
  h.replies.push({ EnvironmentOperation: { operation: operation({ operationId: "op-1", kind: "check", outputTail: "" }) } })
  h.key("c")
  await h.settle()
  assert.deepEqual(h.requests.find((request) => request.CheckProjectEnvironment), { CheckProjectEnvironment: { projectId: "project-1", operationId: "op-1", withAgent: false } })
  const proposals = [{ id: "m", field: "mcp", change: "remove", text: "github", evidence: { path: ".mcp.json", excerpt: "file missing" } }] as const
  h.replies.push({ ProjectEnvironment: { environment: environment({ proposals }) } })
  await h.release({ EnvironmentOperation: { operation: operation({ operationId: "op-1", kind: "check", state: "succeeded", updates: 2, endedAtMs: NOW + 1, outputTail: "" }) } })
  assert.match(h.text(), /− github\s+Proposed · \.mcp\.json/)
})
