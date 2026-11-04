import assert from "node:assert/strict"
import test from "node:test"

import {
  cancelEnvironmentOperationRequest,
  checkProjectEnvironmentRequest,
  draftProjectEnvironmentRequest,
  environmentOperationFromResponse,
  getEnvironmentOperationRequest,
  getProjectEnvironmentRequest,
  importProjectEnvironmentEntriesRequest,
  parseEnvironmentOperationView,
  remoteEnvironmentOperationRequest,
  resolveEnvironmentProposalRequest,
  runRemoteProjectEnvironmentSetupRequest,
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
  mcps: [{ name: "github", source: ".mcp.json", presence: "present" }, { name: "notion", source: ".mcp.json", presence: "missing" }],
  skills: [{ name: "dataviz", source: ".claude/skills/dataviz/SKILL.md", presence: "present" }],
  changedAtMs: 1_700_000_000_000,
  machineLabel: "Miguel's Mac",
  checkoutPath: "/work/project",
  setupState: { state: "failed", operationId: "op-1", endedAtMs: 1_700_000_100_000, exitCode: 2, reason: null },
  proposals: [{ id: "p1", field: "secret", change: "add", text: "STRIPE_SECRET_KEY", evidence: { path: "lib/billing.ts", excerpt: "process.env.STRIPE_SECRET_KEY" } }],
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
  updates: 4,
  outputTail: "installing\n",
  entries: [],
  sourceLabel: null,
}

test("Environment v2 requests use the wire names and drop absent checkoutPath", () => {
  assert.equal(projectEnvironmentMinimumProtocolVersion, 973)
  assert.deepEqual(getProjectEnvironmentRequest("p"), { GetProjectEnvironment: { projectId: "p" } })
  assert.deepEqual(getProjectEnvironmentRequest("p", "/work/p"), { GetProjectEnvironment: { projectId: "p", checkoutPath: "/work/p" } })
  assert.deepEqual(runProjectEnvironmentSetupRequest("p", "op"), { RunProjectEnvironmentSetup: { projectId: "p", operationId: "op" } })
  assert.deepEqual(runProjectEnvironmentSetupRequest("p", "op", "/work/p"), { RunProjectEnvironmentSetup: { projectId: "p", operationId: "op", checkoutPath: "/work/p" } })
  assert.deepEqual(getEnvironmentOperationRequest("op"), { GetEnvironmentOperation: { operationId: "op" } })
  assert.deepEqual(getEnvironmentOperationRequest("op", 0), { GetEnvironmentOperation: { operationId: "op", waitAfterUpdates: 0 } })
  assert.deepEqual(cancelEnvironmentOperationRequest("op"), { CancelEnvironmentOperation: { operationId: "op" } })
  assert.deepEqual(setProjectEnvironmentSecretRequest("p", "API_KEY", "v"), { SetProjectEnvironmentSecret: { projectId: "p", name: "API_KEY", value: "v" } })
})

test("Import names its source and leaves out entries for Update; Draft and Check name only the Project", () => {
  assert.deepEqual(importProjectEnvironmentEntriesRequest({ projectId: "p", operationId: "op", sourceKernelId: "k" }), { ImportProjectEnvironmentEntries: { projectId: "p", operationId: "op", sourceKernelId: "k" } })
  assert.deepEqual(importProjectEnvironmentEntriesRequest({ projectId: "p", operationId: "op", sourceKernelId: "k", entries: [{ kind: "secret", name: "T" }], checkoutPath: "/w" }), {
    ImportProjectEnvironmentEntries: { projectId: "p", operationId: "op", sourceKernelId: "k", entries: [{ kind: "secret", name: "T" }], checkoutPath: "/w" },
  })
  assert.deepEqual(draftProjectEnvironmentRequest("p", "op"), { DraftProjectEnvironment: { projectId: "p", operationId: "op" } })
})

test("Setup on another kernel names the target; its operation is read or stopped there", () => {
  assert.deepEqual(runRemoteProjectEnvironmentSetupRequest("p", "op", "k"), { RunRemoteProjectEnvironmentSetup: { projectId: "p", operationId: "op", targetKernelId: "k" } })
  assert.deepEqual(runRemoteProjectEnvironmentSetupRequest("p", "op", "k", "/w"), { RunRemoteProjectEnvironmentSetup: { projectId: "p", operationId: "op", targetKernelId: "k", checkoutPath: "/w" } })
  assert.deepEqual(remoteEnvironmentOperationRequest("op", "k"), { RemoteEnvironmentOperation: { operationId: "op", targetKernelId: "k" } })
  assert.deepEqual(remoteEnvironmentOperationRequest("op", "k", true), { RemoteEnvironmentOperation: { operationId: "op", targetKernelId: "k", stop: true } })
})

test("Check names only the Project and a proposal is accepted or dismissed by its id; proposals read with their evidence", () => {
  assert.deepEqual(checkProjectEnvironmentRequest("p", "op", true), { CheckProjectEnvironment: { projectId: "p", operationId: "op", withAgent: true } })
  assert.deepEqual(checkProjectEnvironmentRequest("p", "op", false), { CheckProjectEnvironment: { projectId: "p", operationId: "op", withAgent: false } })
  assert.deepEqual(resolveEnvironmentProposalRequest("p", "p1", true), { ResolveEnvironmentProposal: { projectId: "p", proposalId: "p1", accept: true } })
  assert.deepEqual(projectEnvironmentFromResponse({ ProjectEnvironment: { environment: view } }).proposals, view.proposals)
  assert.throws(() => projectEnvironmentFromResponse({ ProjectEnvironment: { environment: { ...view, proposals: [{ ...view.proposals[0], field: "tool" }] } } }), /field/)
})

test("an Import operation reads with its entries and source", () => {
  const parsed = parseEnvironmentOperationView({ ...operation, kind: "import", sourceLabel: "Mac", entries: [{ kind: "file", name: ".env", state: "failed", reason: "Source offline" }] })
  assert.equal(parsed.kind, "import")
  assert.equal(parsed.sourceLabel, "Mac")
  assert.deepEqual(parsed.entries, [{ kind: "file", name: ".env", state: "failed", reason: "Source offline" }])
  assert.throws(() => parseEnvironmentOperationView({ ...operation, entries: [{ kind: "mcp", name: "x", state: "failed", reason: null }] }), /kind/)
})

test("SaveProjectEnvironment carries the revision and wraps names and paths as rows", () => {
  assert.deepEqual(saveProjectEnvironmentRequest({
    projectId: "p", expectedRevision: 4, script: "make", variables: "A=1", secretNames: ["TOKEN"], filePaths: [".env"],
    mcps: [{ name: "github", source: ".mcp.json" }], skills: [],
  }), {
    SaveProjectEnvironment: {
      projectId: "p", expectedRevision: 4, setup: { script: "make" }, variables: "A=1",
      secrets: [{ name: "TOKEN" }], files: [{ path: ".env" }], mcps: [{ name: "github", source: ".mcp.json" }], skills: [],
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
    { state: "ok", operationId: "op-1", endedAtMs: 5 },
    { state: "cancelled", operationId: "op-1", endedAtMs: 5 },
    { state: "failed", operationId: "op-1", endedAtMs: 5, exitCode: null, reason: "Kernel restarted" },
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
  assert.throws(() => projectEnvironmentFromResponse(broken({ setupState: { state: "ok", operationId: "op-1" } })), /missing "endedAtMs"/)
  assert.throws(() => environmentOperationFromResponse({ EnvironmentOperation: { operation: { ...operation, outputTail: 1 } } }), /"outputTail" must be text/)
})
