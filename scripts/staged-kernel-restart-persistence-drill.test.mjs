import assert from "node:assert/strict"
import test from "node:test"

import {
  assertNoDuplicateSessionAgents,
  assertRestoredSessionIdentity,
  createPublicRequestLedger,
  durableAgentFingerprint,
  durableSessionFingerprint,
  STAGED_RESTART_PUBLIC_REQUEST_COUNT,
  stopOwnedChild,
} from "./staged-kernel-restart-persistence-drill-helpers.mjs"

const binary = "/opt/staged/chariox-kernel"

function agentFixture(overrides = {}) {
  return {
    id: "agent-1",
    agent_ref: "agent-ref-1",
    session_id: "session-1",
    provider: "none",
    model: null,
    alias: null,
    worktree_id: "worktree-1",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 100,
    ...overrides,
  }
}

function sessionFixture(overrides = {}) {
  const agent = agentFixture(overrides.agent)
  return {
    id: "session-1",
    project_id: "project-1",
    alias: "staged-kernel-restart",
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    host_machine_id: "machine-1",
    host_daemon_id: "daemon-1",
    created_at_ms: 100,
    status: "Active",
    agent_defaults: null,
    focused_agent_id: null,
    max_agents: 1,
    config_state: { version: 0, values: {} },
    workspace_live_sync_mode: null,
    agents: [agent],
    ...overrides,
  }
}

function childFixture() {
  const handlers = new Map()
  const signals = []
  const child = {
    pid: 4242,
    spawnfile: binary,
    spawnargs: [binary],
    exitCode: null,
    signalCode: null,
    kill(signal) {
      signals.push(signal)
      return true
    },
    once(event, handler) {
      handlers.set(event, handler)
      return child
    },
  }
  return { child, signals, handlers }
}

function ownershipFixture() {
  return {
    schema_version: 1,
    run_id: "run-1",
    generation: 1,
    phase: "ready",
    pid: 4242,
    binary,
    process_start_time_ticks: "100",
    root_dir: "/tmp/chariox-staged-kernel-restart-run-1",
    chariox_home: "/tmp/chariox-staged-kernel-restart-run-1/chariox-home",
    workspace: "/tmp/chariox-staged-kernel-restart-run-1/workspace",
    token_file: "/tmp/chariox-staged-kernel-restart-run-1/auth-token",
    endpoint: "ws://127.0.0.1:43118",
    ownership_file: "/tmp/chariox-staged-kernel-restart-run-1/child-ownership.json",
  }
}

test("restart ownership maps persisted start identity and refuses a replaced PID without signaling", async () => {
  const { child, signals } = childFixture()
  const ownership = ownershipFixture()
  await assert.rejects(
    () => stopOwnedChild(child, ownership, {
      readProcessIdentity: async () => ({
        pid: 4242,
        executable: binary,
        argv: [binary],
        process_start_time_ticks: "101",
      }),
      graceMs: 1,
      killGraceMs: 1,
    }),
    /owned child start identity changed.*expected 100, got 101/,
  )
  assert.deepEqual(signals, [], "a replaced PID must not receive a signal")
})

test("child cleanup failure is a hard failure after bounded TERM and KILL attempts", async () => {
  const { child, signals } = childFixture()
  const ownership = ownershipFixture()
  await assert.rejects(
    () => stopOwnedChild(child, ownership, {
      verify: async () => {},
      graceMs: 1,
      killGraceMs: 1,
    }),
    /owned child cleanup failed/,
  )
  assert.deepEqual(signals, ["SIGTERM", "SIGKILL"])
})

test("runnable restart orchestration counts the reattached session list", () => {
  const requestLabels = [
    "first daemon health",
    "initial session list",
    "session create",
    "first session attach",
    "first session state",
    "first session detach",
    "detached session state",
    "restarted daemon health",
    "post-restart session list",
    "post-restart session resolve",
    "post-restart session state",
    "post-restart session attach",
    "post-restart reattached state",
    "post-restart reattached session list",
    "post-restart session detach",
    "final session state",
  ]
  assert.equal(requestLabels.length, STAGED_RESTART_PUBLIC_REQUEST_COUNT)
  const complete = createPublicRequestLedger()
  for (const label of requestLabels) complete.record(label)
  assert.equal(complete.assertComplete(), STAGED_RESTART_PUBLIC_REQUEST_COUNT)

  const missingReattachedList = createPublicRequestLedger()
  for (const label of requestLabels.filter((label) => label !== "post-restart reattached session list")) {
    missingReattachedList.record(label)
  }
  assert.throws(
    () => missingReattachedList.assertComplete(),
    /exactly 16 public requests, got 15.*post-restart reattached state/,
  )
})

test("restored-session orchestration rejects a lost session", () => {
  const session = sessionFixture()
  const agent = session.agents[0]
  assert.throws(
    () => assertRestoredSessionIdentity({
      listedSessions: [],
      resolvedSession: null,
      stateSession: null,
      expectedSession: durableSessionFingerprint(session),
      expectedAgent: durableAgentFingerprint(agent),
    }),
    /created session session-1 was lost after process restart/,
  )
})

test("restored-session orchestration rejects a replacement with the same session id", () => {
  const expected = sessionFixture()
  const replacement = sessionFixture({ project_id: "replacement-project" })
  const agent = expected.agents[0]
  assert.throws(
    () => assertRestoredSessionIdentity({
      listedSessions: [replacement],
      resolvedSession: replacement,
      stateSession: replacement,
      expectedSession: durableSessionFingerprint(expected),
      expectedAgent: durableAgentFingerprint(agent),
    }),
    /created session session-1 was replaced after process restart/,
  )
})

test("reattachment keeps exactly the created agent identity", () => {
  const session = sessionFixture()
  assert.equal(assertNoDuplicateSessionAgents(session, "agent-1"), true)
  assert.throws(
    () => assertNoDuplicateSessionAgents({ ...session, agents: [...session.agents, session.agents[0]] }, "agent-1"),
    /duplicate agent identities/,
  )
})
