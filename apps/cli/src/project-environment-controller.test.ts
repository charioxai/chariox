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
    startedAtMs: NOW, endedAtMs: null, exitCode: null, reason: null, outputTail: "installing\n", entries: [], sourceLabel: null, ...patch,
  }
}

type Reply = unknown | Error | (() => unknown)

function harness(initial: ProjectEnvironmentView = environment()) {
  const requests: Record<string, any>[] = []
  const replies: Reply[] = []
  const scheduled: Array<{ callback: () => void; delayMs: number }> = []
  let renders = 0
  let operationIds = 0
  const controller = createProjectEnvironmentController({
    send: async (request) => {
      requests.push(request as Record<string, any>)
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
  return { controller, requests, replies, scheduled, key, type, text, open, settle, focusOn, renders: () => renders }
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
    secrets: [{ name: "API_KEY" }, { name: "DB_URL" }], files: [{ path: ".env.local" }, { path: "certs/dev.pem" }],
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

for (const dialog of ["open", "closed", "reopened", "another Project"]) {
  for (const nextEdit of ["variables", "same script", "original script"]) {
    test(`autosave keeps a pending ${nextEdit} edit with the dialog ${dialog}`, async () => {
      const h = harness()
      await h.open()
      let firstReply!: (value: unknown) => void
      let secondReply!: (value: unknown) => void
      h.replies.push(new Promise((resolve) => { firstReply = resolve }))
      h.key("down")
      h.key("return")
      h.type(" && make test")
      h.key("escape")
      assert.equal(h.requests.length, 2)

      if (nextEdit === "variables") h.key("down")
      h.key("return")
      if (nextEdit === "original script") {
        for (let i = 0; i < " && make test".length; i++) h.key("backspace")
      } else h.type(nextEdit === "variables" ? "&DEBUG=1" : " && make lint")
      h.key("escape")
      assert.equal(h.requests.length, 2, "the second save waits for the first revision")

      if (dialog !== "open") {
        h.key("escape")
        assert.equal(h.controller.isOpen(), false)
        if (dialog === "reopened") await h.controller.open("project-1")
        else if (dialog === "another Project") {
          h.replies.push({ ProjectEnvironment: { environment: environment({ projectId: "project-2", setup: { script: "other setup" } }) } })
          await h.controller.open("project-2")
        }
      }

      const expectedScript = nextEdit === "variables" ? "make setup && make test"
        : nextEdit === "same script" ? "make setup && make test && make lint" : "make setup"
      const expectedVariables = nextEdit === "variables" ? "PORT=3000&DEBUG=1" : "PORT=3000"
      h.replies.push(new Promise((resolve) => { secondReply = resolve }))
      firstReply({ ProjectEnvironmentSaved: { environment: environment({ revision: 3, setup: { script: "make setup && make test" } }) } })
      await h.settle()
      if (dialog !== "another Project") {
        assert(h.text().includes(expectedScript), "the first reply keeps newer script text")
        assert(h.text().includes(expectedVariables), "the first reply keeps newer variables")
      } else assert(h.text().includes("other setup"), "the first Project cannot overwrite the second")
      const expectedRequests = dialog === "another Project" ? 4 : 3
      assert.equal(h.requests.length, expectedRequests, "a follow-up save sends the retained edit")
      assert.deepEqual(h.requests.at(-1)!.SaveProjectEnvironment, {
        projectId: "project-1", expectedRevision: 3, setup: { script: expectedScript }, variables: expectedVariables,
        secrets: [{ name: "API_KEY" }, { name: "DB_URL" }], files: [{ path: ".env.local" }, { path: "certs/dev.pem" }],
      })
      if (dialog !== "another Project") assert(h.text().includes("Unsaved"), "the newer edit waits for its own acknowledgement")
      secondReply({ ProjectEnvironmentSaved: { environment: environment({ revision: 4, setup: { script: expectedScript }, variables: expectedVariables }) } })
      await h.settle()
      assert(!h.text().includes("Unsaved"))
      assert.equal(h.requests.length, expectedRequests)
      const saved = environment({ revision: 4, setup: { script: expectedScript }, variables: expectedVariables })
      if (dialog !== "open") {
        h.controller.close()
        h.replies.push({ ProjectEnvironment: { environment: saved } })
        await h.controller.open("project-1")
        assert(h.text().includes(expectedScript))
        assert(h.text().includes(expectedVariables))
        assert(!h.text().includes("Unsaved"))
      }
    })
  }
}

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

test("Run setup starts an operation, polls once a second and shows the output tail", async () => {
  const h = harness()
  await h.open()
  h.replies.push({ EnvironmentOperation: { operation: operation() } })
  h.replies.push({ ProjectEnvironment: { environment: environment({ setupState: { state: "running", operationId: "op-1", startedAtMs: NOW } }) } })
  h.key("return")
  await h.settle()
  assert.deepEqual(h.requests[1], { RunProjectEnvironmentSetup: { projectId: "project-1", operationId: "op-1" } })
  assert.match(h.text(), /Miguel's Mac · Running\s+Stop/)
  assert(h.text().includes("  installing"))
  assert.deepEqual(h.scheduled.map((entry) => entry.delayMs), [1000])

  h.replies.push({ EnvironmentOperation: { operation: operation({ outputTail: "installing\ncompiling\n" }) } })
  h.scheduled.shift()!.callback()
  await h.settle()
  assert.deepEqual(h.requests.at(-1), { GetEnvironmentOperation: { operationId: "op-1" } })
  assert(h.text().includes("  compiling"))
  assert.deepEqual(h.scheduled.map((entry) => entry.delayMs), [1000])

  h.replies.push({ EnvironmentOperation: { operation: operation({ state: "succeeded", endedAtMs: NOW + 5, exitCode: 0, outputTail: "installing\ncompiling\ndone\n" }) } })
  h.replies.push({ ProjectEnvironment: { environment: environment({ setupState: { state: "ok", operationId: "op-1", endedAtMs: NOW + 5 } }) } })
  h.scheduled.shift()!.callback()
  await h.settle()
  assert.deepEqual(h.requests.at(-1), { GetProjectEnvironment: { projectId: "project-1" } })
  assert.match(h.text(), /Setup ran just now\s+Run setup/)
  assert.equal(h.scheduled.length, 0)
})

test("Stop cancels the running operation and the record shows cancelled", async () => {
  const running = environment({ setupState: { state: "running", operationId: "op-9", startedAtMs: NOW } })
  const h = harness(running)
  await h.open()
  assert.deepEqual(h.scheduled.map((entry) => entry.delayMs), [1000])
  h.replies.push({ EnvironmentOperation: { operation: operation({ operationId: "op-9", state: "cancelled", endedAtMs: NOW + 1 }) } })
  h.replies.push({ ProjectEnvironment: { environment: environment({ setupState: { state: "cancelled", operationId: "op-9", endedAtMs: NOW + 1 } }) } })
  h.key("return")
  await h.settle()
  assert.deepEqual(h.requests[1], { CancelEnvironmentOperation: { operationId: "op-9" } })
  assert.match(h.text(), /Setup cancelled just now\s+Run setup/)
  h.replies.push(operation())
  h.scheduled.shift()!.callback()
  await h.settle()
  assert.equal(h.requests.length, 3, "a stopped poll sends nothing")
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
