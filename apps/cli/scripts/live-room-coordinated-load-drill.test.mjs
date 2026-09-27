import assert from "node:assert/strict"
import test from "node:test"

import {
  COORDINATED_LOAD_SCHEMA,
  parseCoordinatedLoadArgs,
  validateCoordinatedLoadConfig,
} from "./lib/room-coordinated-load-plan.mjs"
import { runRoomCoordinatedLoad } from "./lib/room-coordinated-load-runner.mjs"
import { runCoordinatedLoadCli } from "./live-room-coordinated-load-drill.mjs"

const repoRoot = "/source/chariox"

function config(overrides = {}) {
  return {
    schema: COORDINATED_LOAD_SCHEMA,
    runId: "test-run-123",
    approval: { reference: "change-123", approvedMaxHeadedSlices: 1 },
    homeKernel: { url: "ws://127.0.0.1:4100", kernelId: "kernel-local", machineId: "machine-local" },
    relay: { url: "ws://localhost:4200", targetDaemonId: "kernel-local", tokenEnv: "CHARIOX_DRILL_H_RELAY_TOKEN" },
    headedSlices: [{
      sliceId: "slice-1",
      sliceName: "drillh-test-run-123-one",
      roomId: "room-1",
      environmentId: "environment-1",
      runtimeGeneration: 2,
      workerKernelId: "kernel-local",
      workerMachineId: "machine-local",
      ports: {
        codex: 44000,
        opencode: 44300,
        kernel: 44600,
        mcp: 44900,
        relay: 45200,
        novnc: 45500,
        codex_range_start: 46000,
        opencode_range_start: 51200,
      },
    }],
    workflow: { roomId: "room-1", workflowId: "workflow-1", endpointId: "endpoint-1", prompt: "Run the prepared workflow" },
    workspace: { path: "/tmp/drill-h-workspace", worktree: "/tmp/drill-h-worktree" },
    limits: { durationMs: 5_000, sampleIntervalMs: 1_000, maxKernelLatencyMs: 2_000, maxMemoryBytes: 1_000_000_000, maxCpuPercent: 500 },
    slowViewer: { viewerId: "web-local", afterSample: 1, delayMs: 100 },
    evidenceRoot: "/tmp/drill-h-evidence",
    ...overrides,
  }
}

function plan() {
  return validateCoordinatedLoadConfig(config(), { repoRoot })
}

function fixtureRuntime({ incompleteCleanup = false, workflowStatus = "running" } = {}) {
  const calls = []
  let clock = 0
  const snapshot = {
    approvedMaxHeadedSlices: 1,
    slices: [{
      sliceId: "slice-1",
      sliceName: "drillh-test-run-123-one",
      roomId: "room-1",
      environmentId: "environment-1",
      runtimeGeneration: 2,
      displayMode: "headed",
      sliceStatus: "running",
      environmentLifecycle: "ready",
      workerKernelId: "kernel-local",
      workerMachineId: "machine-local",
    }],
  }
  const inventory = (phase) => ({
    ownedProcessIds: [],
    containers: ["container-1|chariox-slice-drillh-test-run-123-one|Up|127.0.0.1:44600->53119/tcp"],
    volumes: ["chariox-slice-drillh-test-run-123-one-home"],
    ports: ["44600:123"],
    remainingOwnedPids: phase === "after" && incompleteCleanup ? ["999"] : [],
  })
  function handle(kind, id, extras = {}) {
    return { ownerRunId: "test-run-123", kind, id, started: true, stopToken: `${kind}:${id}`, ...extras }
  }
  return {
    calls,
    now: () => clock,
    sleep: async (milliseconds) => { clock += milliseconds },
    captureInventory: async (_plan, { phase }) => { calls.push(`inventory:${phase}`); return inventory(phase) },
    verifyPrepared: async () => { calls.push("verify"); return snapshot },
    startViewer: async (_plan, viewer) => { calls.push(`start:${viewer.id}`); return handle("viewer", viewer.id) },
    startTui: async (_plan, tui) => { calls.push(`start:${tui.id}`); return handle("tui", tui.id, { pid: tui.id === "local-tui" ? 101 : 102, processGroupId: tui.id === "local-tui" ? 101 : 102 }) },
    startWorkflow: async () => { calls.push("start:workflow"); return handle("workflow", "workflow-1", { workflowRunId: "workflow-run-1" }) },
    injectSlowViewer: async (task, delayMs) => {
      calls.push(`slow:${task.id}`)
      return { viewerId: task.id, requestedDelayMs: delayMs, observedDelayMs: delayMs }
    },
    sample: async (_plan, { sampleIndex }) => {
      calls.push(`sample:${sampleIndex}`)
      return {
        sampleIndex,
        capturedAt: new Date(1_000 + sampleIndex).toISOString(),
        elapsedMs: clock,
        localKernelLatencyMs: 8,
        relayKernelLatencyMs: 12,
        aggregateContainerMemoryBytes: 50_000_000,
        aggregateContainerCpuPercent: 10,
        viewerLocalFrameBytes: 1024,
        viewerRelayFrameBytes: 2048,
        hostFreeMemoryBytes: 9_000_000_000,
        processRssBytes: 400_000_000,
        ownedProcessCount: 2,
        openListenerCount: 4,
        workflowStatus,
      }
    },
    stopOwnedTask: async (task) => {
      calls.push(`stop:${task.kind}:${task.id}`)
      return { ownerRunId: task.ownerRunId, stopped: true, remaining: incompleteCleanup && task.id === "web-relay" }
    },
  }
}

test("config requires exact approved identities and bounded local endpoints", () => {
  const parsed = validateCoordinatedLoadConfig(config(), { repoRoot })
  assert.equal(parsed.headedSlices.length, parsed.approval.approvedMaxHeadedSlices)
  assert.equal(parsed.headedSlices[0].publishedPorts.length, 45)
  assert.throws(() => validateCoordinatedLoadConfig(config({ operatorPassed: true }), { repoRoot }), /unsupported fields/)
  assert.throws(() => validateCoordinatedLoadConfig(config({
    relay: { ...config().relay, url: "wss://relay.example.test" },
  }), { repoRoot }), /loopback/)
  assert.throws(() => validateCoordinatedLoadConfig(config({
    approval: { reference: "change-123", approvedMaxHeadedSlices: 2 },
  }), { repoRoot }), /exactly match/)
})

test("argument parser requires one explicit mode and config", () => {
  assert.equal(parseCoordinatedLoadArgs(["--validate-config", "--config", "/tmp/config.json"]).mode, "validate")
  assert.equal(parseCoordinatedLoadArgs(["--execute-live", "--config", "/tmp/config.json"]).mode, "execute")
  assert.throws(() => parseCoordinatedLoadArgs(["--config", "/tmp/config.json"]), /select exactly one/)
  assert.throws(() => parseCoordinatedLoadArgs(["--execute-live", "--validate-config", "--config", "/tmp/config.json"]), /select exactly one/)
})

test("measured run starts both viewers and TUIs before workflow, delays once, then stops only owned tasks in reverse", async () => {
  const runtime = fixtureRuntime()
  const report = await runRoomCoordinatedLoad(plan(), runtime)
  assert.equal(report.status, "measured")
  assert.equal(report.acceptance, "not_proven")
  assert.equal(report.workflow.acceptedByKernel, true)
  assert.equal(report.workflow.workflowRunId, "workflow-run-1")
  assert.equal(report.workflow.executionObserved, true)
  assert.equal(report.timing.observedElapsedMs, report.timing.requestedDurationMs)
  assert.equal(report.slowViewer.injectedOnce, true)
  assert.equal(report.cleanup.clean, true)
  assert.equal(Object.hasOwn(report, "passed"), false)
  assert.deepEqual(runtime.calls.slice(0, 7), [
    "inventory:before", "verify", "start:web-local", "start:web-relay", "start:local-tui", "start:relay-tui", "start:workflow",
  ])
  assert.equal(runtime.calls.filter((call) => call === "slow:web-local").length, 1)
  assert.deepEqual(runtime.calls.slice(-6), [
    "stop:workflow:workflow-1", "stop:tui:relay-tui", "stop:tui:local-tui",
    "stop:viewer:web-relay", "stop:viewer:web-local", "inventory:after",
  ])
})

test("cleanup failure is retained as failed measurement and never becomes acceptance", async () => {
  const report = await runRoomCoordinatedLoad(plan(), fixtureRuntime({ incompleteCleanup: true }))
  assert.equal(report.status, "failed")
  assert.equal(report.acceptance, "not_proven")
  assert.equal(report.failureCode, "owned_task_cleanup_incomplete")
  assert.equal(report.cleanup.clean, false)
  assert.deepEqual(report.cleanup.remainingOwnedPids, ["999"])
})

test("an invoked workflow that never progresses does not produce a measured result", async () => {
  const report = await runRoomCoordinatedLoad(plan(), fixtureRuntime({ workflowStatus: "waiting" }))
  assert.equal(report.status, "failed")
  assert.equal(report.failureCode, "workflow_execution_not_observed")
  assert.equal(report.acceptance, "not_proven")
})

test("help remains source-only and states that live acceptance is not proven", async () => {
  const output = []
  const exitCode = await runCoordinatedLoadCli(["--help"], { log: (line) => output.push(line), error: (line) => output.push(line) })
  assert.equal(exitCode, 0)
  assert.match(output.join("\n"), /not an acceptance verdict/)
})
