import assert from "node:assert/strict"
import test from "node:test"

import {
  cancelEnvironmentOperationRequest,
  environmentOperationFromResponse,
  getEnvironmentOperationRequest,
  getProjectEnvironmentRequest,
  projectEnvironmentFromResponse,
  projectEnvironmentMinimumProtocolVersion,
  projectEnvironmentSaveOutcomeFromResponse,
  projectEnvironmentSavedFromResponse,
  runProjectEnvironmentSetupRequest,
  saveProjectEnvironmentRequest,
  setProjectEnvironmentSecretRequest,
} from "./ipc-project-environment-requests.js"

const view = {
  projectId: "project-1",
  exists: true,
  revision: 3,
  setup: { script: "pnpm install" },
  variables: "PORT=3000\n",
  secrets: [{ name: "API_KEY", presence: "locked" }, { name: "DB_URL", presence: "present" }],
  files: [{ path: ".env.local", presence: "missing", sizeBytes: null }, { path: "certs/dev.pem", presence: "present", sizeBytes: 1200 }],
  changedAtMs: 1_700_000_000_000,
  machineLabel: "Miguel's Mac",
  checkoutPath: "/work/project",
  setupState: { state: "failed", endedAtMs: 1_700_000_100_000, exitCode: 2, reason: null },
}

const operation = {
  operationId: "op-1",
  projectId: "project-1",
  checkoutPath: "/work/project",
  kind: "setup",
  state: "running",
  startedAtMs: 10,
  endedAtMs: null,
  exitCode: null,
  reason: null,
  outputTail: "installing\n",
}

test("Environment v2 requests use the wire names and drop absent checkoutPath", () => {
  assert.equal(projectEnvironmentMinimumProtocolVersion, 972)
  assert.deepEqual(getProjectEnvironmentRequest("p"), { GetProjectEnvironment: { projectId: "p" } })
  assert.deepEqual(getProjectEnvironmentRequest("p", "/work/p"), { GetProjectEnvironment: { projectId: "p", checkoutPath: "/work/p" } })
  assert.deepEqual(runProjectEnvironmentSetupRequest("p", "op"), { RunProjectEnvironmentSetup: { projectId: "p", operationId: "op" } })
  assert.deepEqual(runProjectEnvironmentSetupRequest("p", "op", "/work/p"), { RunProjectEnvironmentSetup: { projectId: "p", operationId: "op", checkoutPath: "/work/p" } })
  assert.deepEqual(getEnvironmentOperationRequest("op"), { GetEnvironmentOperation: { operationId: "op" } })
  assert.deepEqual(cancelEnvironmentOperationRequest("op"), { CancelEnvironmentOperation: { operationId: "op" } })
  assert.deepEqual(setProjectEnvironmentSecretRequest("p", "API_KEY", "v"), { SetProjectEnvironmentSecret: { projectId: "p", name: "API_KEY", value: "v" } })
})

test("SaveProjectEnvironment carries the revision and wraps names and paths as rows", () => {
  assert.deepEqual(saveProjectEnvironmentRequest({
    projectId: "p", expectedRevision: 4, script: "make", variables: "A=1", secretNames: ["TOKEN"], filePaths: [".env"],
  }), {
    SaveProjectEnvironment: {
      projectId: "p", expectedRevision: 4, setup: { script: "make" }, variables: "A=1",
      secrets: [{ name: "TOKEN" }], files: [{ path: ".env" }],
    },
  })
})

test("responses parse into the typed views", () => {
  assert.deepEqual(projectEnvironmentFromResponse({ ProjectEnvironment: { environment: view } }), view)
  assert.deepEqual(projectEnvironmentSavedFromResponse({ ProjectEnvironmentSaved: { environment: view } }), view)
  assert.deepEqual(projectEnvironmentSaveOutcomeFromResponse({ ProjectEnvironmentSaved: { environment: view } }), { kind: "saved", environment: view })
  assert.deepEqual(projectEnvironmentSaveOutcomeFromResponse({ ProjectEnvironmentConflict: { environment: view } }), { kind: "conflict", environment: view })
  assert.deepEqual(environmentOperationFromResponse({ EnvironmentOperation: { operation } }), operation)
})

test("every setup state parses", () => {
  const states = [
    { state: "notRun" },
    { state: "ok", endedAtMs: 5 },
    { state: "cancelled", endedAtMs: 5 },
    { state: "failed", endedAtMs: 5, exitCode: null, reason: "Kernel restarted" },
    { state: "running", operationId: "op-1", startedAtMs: 5 },
  ]
  for (const setupState of states) {
    assert.deepEqual(projectEnvironmentFromResponse({ ProjectEnvironment: { environment: { ...view, setupState } } }).setupState, setupState)
  }
})

test("a response of another kind is refused with a clear reason", () => {
  assert.throws(() => projectEnvironmentFromResponse({ Pong: {} }), /did not answer with a Project Environment/)
  assert.throws(() => projectEnvironmentSaveOutcomeFromResponse({ ProjectEnvironment: { environment: view } }), /did not answer the Environment save/)
  assert.throws(() => projectEnvironmentSavedFromResponse({ ProjectEnvironmentConflict: { environment: view } }), /did not confirm the secret/)
  assert.throws(() => environmentOperationFromResponse(null), /did not answer with an Environment operation/)
})

test("a malformed record names the field instead of rendering a guess", () => {
  const broken = (patch: Record<string, unknown>) => ({ ProjectEnvironment: { environment: { ...view, ...patch } } })
  assert.throws(() => projectEnvironmentFromResponse(broken({ revision: -1 })), /"revision" must be a non-negative integer/)
  assert.throws(() => projectEnvironmentFromResponse(broken({ secrets: [{ name: "A", presence: "maybe" }] })), /"presence" is not one of/)
  assert.throws(() => projectEnvironmentFromResponse(broken({ setupState: { state: "paused" } })), /"state" is not one of/)
  assert.throws(() => projectEnvironmentFromResponse(broken({ setupState: { state: "ok" } })), /missing "endedAtMs"/)
  assert.throws(() => environmentOperationFromResponse({ EnvironmentOperation: { operation: { ...operation, outputTail: 1 } } }), /"outputTail" must be text/)
})
