import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { test } from "node:test"

import {
  observeManagedOrdinaryProjectSetup,
  PROJECT_SETUP_MAX_STATUS_AGE_MS,
  PROJECT_SETUP_SELECTION_SCHEMA,
  validateProjectSetupProof,
  validateProjectSetupProofCaptureBinding,
} from "./managed-ordinary-project-setup-observer.mjs"

const NOW = 1_800_000_000_000
const VALIDATION_COMMAND = "node --version"

function digest(value) {
  return `sha256:${createHash("sha256").update(value, "utf8").digest("hex")}`
}

function fixture(overrides = {}) {
  const definition = overrides.definition ?? {
    schema_version: 1,
    origin: "user_authored",
    source: "commands",
    target_platform: "linux-x86_64",
    source_path: null,
    setup_steps: [{ kind: "command", command: "npm install" }],
    validation_commands: [VALIDATION_COMMAND],
  }
  const projectDefinitionDigest = digest(JSON.stringify(definition))
  const selection = {
    schema: PROJECT_SETUP_SELECTION_SCHEMA,
    kernel_endpoint: "ws://127.0.0.1:43118/kernel",
    kernel_id: "kernel-home",
    machine_id: "machine-home",
    session_id: "session-1",
    agent_id: "agent-1",
    project_id: "project-1",
    operation_id: "operation-1",
    attempt: 2,
    created_at_ms: NOW - 60_000,
    worker_id: "worker-1",
    worker_kernel_id: "worker-kernel-1",
    platform: "linux-x86_64",
    ...overrides.selection,
  }
  const session = {
    id: "session-1",
    project_id: "project-1",
    agents: [{
      id: "agent-1",
      remote_execution: {
        worker_machine_id: "worker-1",
        worker_kernel_id: "worker-kernel-1",
      },
    }],
    ...overrides.session,
  }
  const status = {
    operation_id: "operation-1",
    project_id: "project-1",
    session_id: "session-1",
    agent_id: "agent-1",
    worker_id: "worker-1",
    platform: "linux-x86_64",
    phase: "ready",
    attempt: 2,
    progress_percent: 100,
    definition_digest: projectDefinitionDigest,
    validation: {
      worker_id: "worker-1",
      platform: "linux-x86_64",
      commands: [{
        command_digest: digest(VALIDATION_COMMAND),
        exit_code: 0,
        stdout_bytes: 4,
        stderr_bytes: 0,
      }],
    },
    created_at_ms: NOW - 60_000,
    updated_at_ms: NOW - 1_000,
    ...overrides.status,
  }
  const project = {
    id: "project-1",
    status: "active",
    environment_definition: definition,
    ...overrides.project,
  }
  const kernelIdentity = {
    daemon_id: "kernel-home",
    machine_id: "machine-home",
    ...overrides.kernel,
  }
  const calls = []
  const requestBuilders = {
    relayStatusRequest: () => ({ RelayStatus: null }),
    getSessionStateRequest: (sessionId) => ({ GetSessionState: { session_id: sessionId } }),
    listProjectsRequest: (includeArchived) => ({ ListProjects: { include_archived: includeArchived } }),
    getProjectEnvironmentSetupStatusRequest: (operationId) => ({
      GetProjectEnvironmentSetupStatus: { operationId },
    }),
  }
  const clientFactory = (endpoint, clientOptions) => {
    calls.push({ endpoint, clientOptions })
    return {
      async send(request) {
        calls.push(request)
        if ("RelayStatus" in request) {
          return { RelayStatus: { status: structuredClone(kernelIdentity) } }
        }
        if ("GetSessionState" in request) {
          return { SessionState: { session: structuredClone(session) } }
        }
        if ("ListProjects" in request) {
          return { ProjectsListed: { projects: [structuredClone(project)] } }
        }
        if ("GetProjectEnvironmentSetupStatus" in request) {
          return { ProjectEnvironmentSetupStatus: { status: structuredClone(status) } }
        }
        throw new Error("unexpected request")
      },
      async close() {
        calls.push("Close")
      },
    }
  }
  return {
    evidence: { selection },
    requestBuilders,
    clientFactory,
    calls,
    status,
    definition,
    projectDefinitionDigest,
  }
}

function observerOptions(value, extra = {}) {
  return {
    clientFactory: value.clientFactory,
    requestBuilders: value.requestBuilders,
    nowMs: () => NOW,
    ...extra,
  }
}

test("Project setup observation binds fresh Ready validation to kernel, session, agent, Project, operation and definition", async () => {
  const value = fixture()
  const observation = await observeManagedOrdinaryProjectSetup(value.evidence, observerOptions(value))

  assert.equal(observation.product_api_observation, "injected-test-transport")
  assert.equal(observation.ready_validation_verified, true)
  assert.equal(observation.status_fresh, true)
  assert.equal(observation.before_after_identity_stable, true)
  assert.equal(observation.definition_identity_verified, true)
  assert.equal(observation.definition_digest, value.projectDefinitionDigest)
  assert.equal(observation.definition_origin, "user_authored")
  assert.equal(observation.operation_attempt, 2)
  assert.equal(observation.validation_receipts.length, 1)
  assert.equal(observation.validation_receipts[0].exit_code, 0)
  assert.equal(observation.before_snapshot_digest, observation.after_snapshot_digest)
  assert.equal(validateProjectSetupProof(observation).code, "public_api_observation_required")
  assert.equal(value.calls.some((entry) => entry?.GetProjectEnvironmentSetupStatus?.operationId === "operation-1"), true)
  assert.equal(value.calls.at(-1), "Close")
  assert.equal(JSON.stringify(observation).includes("127.0.0.1"), false)
})

test("Project setup identity binder requires the proof hashes to match its provider-turn capture", () => {
  const selection = fixture().evidence.selection
  const captureProvenance = {
    boundary: "official-provider-turn",
    observed: true,
    kernel_identity: {
      kernel_id: selection.kernel_id,
      machine_id: selection.machine_id,
      transport: "local-unix-ipc",
    },
    session_id: selection.session_id,
    agent_id: selection.agent_id,
  }
  const proof = {
    home_kernel_identity_fingerprint: digest(`${selection.kernel_id}\0${selection.machine_id}`),
    session_identity_fingerprint: digest(selection.session_id),
    agent_identity_fingerprint: digest(selection.agent_id),
  }

  // This unit checks hash linkage only; full acceptance also requires the production proof validator.
  assert.deepEqual(validateProjectSetupProofCaptureBinding(proof, captureProvenance), { ok: true, code: null })
  for (const field of [
    "home_kernel_identity_fingerprint",
    "session_identity_fingerprint",
    "agent_identity_fingerprint",
  ]) {
    const forged = { ...proof, [field]: digest(`forged:${field}`) }
    assert.deepEqual(validateProjectSetupProofCaptureBinding(forged, captureProvenance), {
      ok: false,
      code: "capture_identity_mismatch",
      field,
    })
  }
})

test("assertion-only environment evidence is rejected before opening a kernel client", async () => {
  const value = fixture()
  const assertionOnly = {
    observed: true,
    project_setup_ok: true,
    project_identity: "externally-asserted-project",
  }
  await assert.rejects(
    observeManagedOrdinaryProjectSetup(assertionOnly, observerOptions(value)),
    { code: "caller_assertion_rejected" },
  )
  assert.equal(value.calls.length, 0)
})

test("caller claims beside a valid selector are rejected before opening a kernel client", async () => {
  const value = fixture()
  await assert.rejects(
    observeManagedOrdinaryProjectSetup({
      selection: value.evidence.selection,
      project_setup_ok: true,
      project_identity: "caller-claim",
    }, observerOptions(value)),
    { code: "caller_assertion_rejected" },
  )
  assert.equal(value.calls.length, 0)
})

test("wrong, prior-attempt, and old Ready operations fail closed", async (context) => {
  const variants = [
    { name: "wrong operation", status: { operation_id: "operation-other" }, code: "operation_identity_mismatch" },
    { name: "wrong operation session", status: { session_id: "session-other" }, code: "operation_binding_mismatch" },
    { name: "wrong operation agent", status: { agent_id: "agent-other" }, code: "operation_binding_mismatch" },
    { name: "wrong operation Project", status: { project_id: "project-other" }, code: "operation_binding_mismatch" },
    { name: "prior attempt", status: { attempt: 1 }, code: "operation_identity_mismatch" },
    { name: "different operation creation time", status: { created_at_ms: NOW - 120_000 }, code: "operation_identity_mismatch" },
    {
      name: "old Ready status",
      status: { updated_at_ms: NOW - PROJECT_SETUP_MAX_STATUS_AGE_MS - 1 },
      code: "operation_status_stale",
    },
  ]
  for (const variant of variants) {
    await context.test(variant.name, async () => {
      const value = fixture({ status: variant.status })
      await assert.rejects(
        observeManagedOrdinaryProjectSetup(value.evidence, observerOptions(value)),
        { code: variant.code },
      )
    })
  }
})

test("wrong kernel, session, agent, and Project bindings fail closed", async (context) => {
  const variants = [
    {
      name: "wrong kernel",
      overrides: { kernel: { daemon_id: "kernel-other" } },
      code: "kernel_identity_mismatch",
    },
    {
      name: "wrong kernel machine",
      overrides: { kernel: { machine_id: "machine-other" } },
      code: "kernel_identity_mismatch",
    },
    {
      name: "wrong session",
      overrides: { session: { id: "session-other" } },
      code: "session_identity_mismatch",
    },
    {
      name: "wrong agent",
      overrides: { session: { agents: [{ id: "agent-other" }] } },
      code: "agent_identity_mismatch",
    },
    {
      name: "agent connected to a different worker and kernel",
      overrides: {
        session: {
          agents: [{
            id: "agent-1",
            remote_execution: { worker_machine_id: "worker-other", worker_kernel_id: "worker-kernel-other" },
          }],
        },
      },
      code: "agent_worker_mismatch",
    },
    {
      name: "agent disconnected from the selected target",
      overrides: { session: { agents: [{ id: "agent-1", remote_execution: null }] } },
      code: "agent_worker_mismatch",
    },
    {
      name: "wrong Project",
      overrides: { session: { project_id: "project-other" } },
      code: "session_identity_mismatch",
    },
    {
      name: "different Project record",
      overrides: { project: { id: "project-other" } },
      code: "project_identity_mismatch",
    },
  ]
  for (const variant of variants) {
    await context.test(variant.name, async () => {
      const value = fixture(variant.overrides)
      await assert.rejects(
        observeManagedOrdinaryProjectSetup(value.evidence, observerOptions(value)),
        { code: variant.code },
      )
    })
  }
})

test("selection without a worker kernel identity cannot produce target-bound evidence", async () => {
  const value = fixture()
  const selection = { ...value.evidence.selection }
  delete selection.worker_kernel_id
  await assert.rejects(
    observeManagedOrdinaryProjectSetup({ selection }, observerOptions(value)),
    { code: "selection_invalid" },
  )
  assert.equal(value.calls.length, 0)
})

test("missing Ready state, validation results, or matching definition identity fail closed", async (context) => {
  const variants = [
    { name: "not Ready", status: { phase: "validating", progress_percent: 90 }, code: "setup_not_ready" },
    { name: "failed", status: { phase: "failed" }, code: "setup_not_ready" },
    { name: "cancelled", status: { phase: "cancelled" }, code: "setup_not_ready" },
    { name: "wrong status worker", status: { worker_id: "worker-other" }, code: "validation_binding_mismatch" },
    { name: "wrong status platform", status: { platform: "linux-aarch64" }, code: "validation_binding_mismatch" },
    { name: "missing validation", status: { validation: null }, code: "validation_missing" },
    { name: "wrong definition digest", status: { definition_digest: digest("other-definition") }, code: "definition_identity_mismatch" },
    { name: "failed command", status: { validation: { worker_id: "worker-1", platform: "linux-x86_64", commands: [{ command_digest: digest(VALIDATION_COMMAND), exit_code: 1, stdout_bytes: 0, stderr_bytes: 1 }] } }, code: "validation_incomplete" },
  ]
  for (const variant of variants) {
    await context.test(variant.name, async () => {
      const value = fixture({ status: variant.status })
      await assert.rejects(
        observeManagedOrdinaryProjectSetup(value.evidence, observerOptions(value)),
        { code: variant.code },
      )
    })
  }
})

test("definition origin diagnostic preserves only values exposed by Project response fixtures", async (context) => {
  for (const origin of ["user_authored", "utility_generated"]) {
    await context.test(origin, async () => {
      const value = fixture({ definition: {
        schema_version: 1,
        origin,
        source: "commands",
        target_platform: "linux-x86_64",
        source_path: null,
        setup_steps: [{ kind: "command", command: "npm install" }],
        validation_commands: [VALIDATION_COMMAND],
      } })
      const observation = await observeManagedOrdinaryProjectSetup(value.evidence, observerOptions(value))
      assert.equal(observation.definition_origin, origin)
      assert.equal(Object.hasOwn(observation, "definition_was_reused"), false)
      assert.equal(Object.hasOwn(observation, "utility_generated_by"), false)
    })
  }
})
