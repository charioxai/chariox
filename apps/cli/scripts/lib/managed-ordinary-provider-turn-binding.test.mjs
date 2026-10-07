import assert from "node:assert/strict"
import { test } from "node:test"

import {
  CAPTURE_PROVENANCE_SCHEMA,
  ProviderTurnBindingError,
  startManagedOrdinaryProviderTurnBinding,
} from "./managed-ordinary-provider-turn-binding.mjs"

const SOCKET_PATH = "/run/chariox/test.sock"
const KERNEL_ID = "kernel-current"
const MACHINE_ID = "machine-current"
const SESSION_ID = "session-owned"
const AGENT_ID = "agent-owned"
const RUN_ID = "provider-run-owned"
const PROMPT_ID = "prompt-owned"
const ATTACHMENT_ID = "attachment-owned"
const BOOT_ID = "b4a8b0e7-0f5b-4fd8-bcd9-ccc1e8b3c5ac"
const COLLECTOR_PID = 450
const PROVIDER_PID = 201
const PROVIDER_EXECUTABLE = Buffer.from("official provider executable bytes")
const PROVIDER_COMMAND_LINE = Buffer.from("/usr/local/bin/codex\u0000app-server\u0000--api-key=fixture-secret")

function procStat(pid, parentPid, startTimeTicks, state = "S") {
  const fields = [state, String(parentPid), ...Array(17).fill("0"), String(startTimeTicks)]
  return `${pid} (codex app-server fixture) ${fields.join(" ")}\n`
}

function requestBuilders() {
  return {
    relayStatusRequest: () => ({ RelayStatus: null }),
    getProviderRunRequest: (providerRunId) => ({ GetProviderRun: { provider_run_id: providerRunId } }),
    getSessionStateRequest: (sessionId) => ({ GetSessionState: { session_id: sessionId } }),
    listProviderProcessesRequest: () => ({ ListProviderProcesses: { provider: null } }),
  }
}

function harness(overrides = {}) {
  const state = {
    providerPid: PROVIDER_PID,
    providerStartTime: 7001,
    providerState: "S",
    providerExecutable: PROVIDER_EXECUTABLE,
    providerExecutableLink: "/usr/local/bin/codex",
    providerCommandLine: PROVIDER_COMMAND_LINE,
    providerCwd: "/workspace/ordinary",
    bootId: BOOT_ID,
    providerList: [{
      process_id: "managed:codex:provider-key",
      provider: "codex",
      process_label: "codex:app-server",
      pid: PROVIDER_PID,
      status: "active",
      owner_session_ids: [SESSION_ID],
      owner_provider_run_ids: [RUN_ID],
    }],
    run: {
      id: RUN_ID,
      session_id: SESSION_ID,
      agent_instance_id: AGENT_ID,
      provider: "codex",
      state: "Running",
      process_label: "codex:app-server",
      pty_program: "/usr/local/bin/codex",
      pty_args: ["app-server", "--api-key=fixture-secret"],
      pty_env: { API_TOKEN: "fixture-secret" },
      runtime_mcp_auth_token: "fixture-secret",
      working_directory: "/workspace/ordinary",
    },
    activeTurn: {
      prompt_id: PROMPT_ID,
      provider_run_id: RUN_ID,
      source_attachment_id: ATTACHMENT_ID,
      prompt_origin: "chariox",
      status: "running",
      phase: "streaming",
    },
    kernelId: KERNEL_ID,
    machineId: MACHINE_ID,
    requests: [],
    closeCount: 0,
    ...overrides,
  }

  const filesystem = {
    async readFile(path) {
      if (path === "/proc/sys/kernel/random/boot_id") return `${state.bootId}\n`
      const statMatch = /^\/proc\/(\d+)\/stat$/.exec(path)
      if (statMatch) {
        const pid = Number(statMatch[1])
        if (pid === COLLECTOR_PID) return procStat(pid, 301, 9001)
        if (pid === 301) return procStat(pid, PROVIDER_PID, 8001)
        if (pid === state.providerPid) return procStat(pid, 1, state.providerStartTime, state.providerState)
        if (pid === PROVIDER_PID) return procStat(pid, 1, 7001)
        if (pid === 1) return procStat(pid, 0, 100)
      }
      if (path === `/proc/${PROVIDER_PID}/exe`) return state.providerExecutable
      if (path === `/proc/${PROVIDER_PID}/cmdline`) return state.providerCommandLine
      throw Object.assign(new Error(`unexpected readFile ${path}`), { code: "ENOENT" })
    },
    async readlink(path) {
      if (path === `/proc/${PROVIDER_PID}/exe`) return state.providerExecutableLink
      if (path === `/proc/${PROVIDER_PID}/cwd`) return state.providerCwd
      throw Object.assign(new Error(`unexpected readlink ${path}`), { code: "ENOENT" })
    },
  }

  const clientFactory = (socketPath) => {
    assert.equal(socketPath, SOCKET_PATH)
    return {
      async send(request) {
        state.requests.push(request)
        if (Object.hasOwn(request, "RelayStatus")) {
          return { RelayStatus: { status: { daemon_id: state.kernelId, machine_id: state.machineId } } }
        }
        if (Object.hasOwn(request, "ListProviderProcesses")) {
          return { ProviderProcessesListed: { processes: state.providerList } }
        }
        if (Object.hasOwn(request, "GetProviderRun")) {
          if (request.GetProviderRun.provider_run_id !== state.run.id) {
            return { error: "provider run missing" }
          }
          return { ProviderRun: { provider_run: state.run } }
        }
        if (Object.hasOwn(request, "GetSessionState")) {
          if (request.GetSessionState.session_id !== SESSION_ID) return { error: "session missing" }
          return {
            SessionState: {
              session: { id: SESSION_ID, agents: [{ id: AGENT_ID }] },
              agent_activity: { [AGENT_ID]: { active_turn: state.activeTurn } },
            },
          }
        }
        throw new Error(`unexpected request ${JSON.stringify(request)}`)
      },
      async close() { state.closeCount += 1 },
    }
  }

  const processApi = {
    platform: "linux",
    pid: COLLECTOR_PID,
    env: {
      CHARIOX_DAEMON_SOCKET: SOCKET_PATH,
      // These assertions are deliberately stale/untrusted. The helper must
      // derive its pass from the process list and kernel turn responses.
      CHARIOX_PARITY_CAPTURE_EVIDENCE_JSON: JSON.stringify({
        observed: true,
        boundary: "official-provider-turn",
        inside_provider_turn: true,
        independent: true,
        kernel_identity: { kernel_id: KERNEL_ID, machine_id: MACHINE_ID },
      }),
    },
  }

  return {
    state,
    filesystem,
    processApi,
    clientFactory,
    requestBuilders: requestBuilders(),
    start(options = {}) {
      return startManagedOrdinaryProviderTurnBinding({
        filesystem,
        processApi,
        expectedProvider: "codex",
        clientFactory,
        requestBuilders: requestBuilders(),
        ...options,
      })
    },
  }
}

test("binds the real collector ancestry to one kernel-owned active provider turn", async () => {
  const h = harness()
  const binding = await h.start()
  const proof = await binding.finish()

  assert.equal(proof.schema, CAPTURE_PROVENANCE_SCHEMA)
  assert.equal(proof.boundary, "official-provider-turn")
  assert.equal(proof.kernel_identity.kernel_id, KERNEL_ID)
  assert.equal(proof.kernel_identity.machine_id, MACHINE_ID)
  assert.equal(proof.kernel_identity.transport, "local-unix-ipc")
  assert.equal(proof.session_id, SESSION_ID)
  assert.equal(proof.agent_id, AGENT_ID)
  assert.equal(proof.attachment_id, ATTACHMENT_ID)
  assert.equal(proof.prompt_id, PROMPT_ID)
  assert.equal(proof.provider.provider_run_id, RUN_ID)
  assert.equal(proof.provider.name, "codex")
  assert.equal(proof.prompt_phase_start, "streaming")
  assert.equal(proof.prompt_phase_end, "streaming")
  assert.equal(proof.process.pid, PROVIDER_PID)
  assert.equal(proof.process.linux_boot_id, BOOT_ID)
  assert.equal(proof.process.start_time_ticks, "7001")
  assert.equal(proof.process.executable_sha256.startsWith("sha256:"), true)
  assert.equal(h.state.closeCount, 2)
  assert.equal(h.state.requests.some((request) => Object.hasOwn(request, "GetProviderRun")), true)
  assert.equal(h.state.requests.some((request) => Object.hasOwn(request, "GetSessionState")), true)
  assert.equal(h.state.requests.some((request) => Object.hasOwn(request, "ListProviderProcesses")), true)
  assert.equal(JSON.stringify(proof).includes("fixture-secret"), false)
})

test("caller capture JSON and environment flags alone cannot establish a provider turn", async () => {
  const h = harness({ providerList: [] })
  await assert.rejects(h.start(), (error) => error instanceof ProviderTurnBindingError
    && error.code === "provider_process_owner_mismatch")
})

test("rejects a non-provider boundary without a separate proof authority", async () => {
  const h = harness()
  await assert.rejects(h.start({ expectedBoundary: "remote-command" }), (error) => error instanceof ProviderTurnBindingError
    && error.code === "provider_turn_boundary_unsupported")
  assert.equal(h.state.requests.length, 0)
})

test("rejects an old running provider when the selected provider differs", async () => {
  const h = harness()
  await assert.rejects(h.start({ expectedProvider: "claude" }), (error) => error instanceof ProviderTurnBindingError
    && error.code === "provider_process_owner_mismatch")
})

test("rejects a provider process PID that is not in the collector parent chain", async () => {
  const h = harness({ providerPid: 202 })
  h.state.providerList = [{
    ...h.state.providerList[0],
    pid: 202,
  }]
  await assert.rejects(h.start(), (error) => error instanceof ProviderTurnBindingError
    && error.code === "provider_process_owner_mismatch")
})

test("rejects mismatched run, session, agent, prompt, and attachment ownership", async (t) => {
  for (const [name, activeTurn] of [
    ["run", { provider_run_id: "provider-run-foreign" }],
    ["prompt origin", { prompt_origin: "external" }],
    ["status", { status: "settling" }],
    ["attachment", { source_attachment_id: null }],
  ]) {
    await t.test(name, async () => {
      const h = harness({ activeTurn: { ...harness().state.activeTurn, ...activeTurn } })
      await assert.rejects(h.start(), (error) => error instanceof ProviderTurnBindingError
        && error.code === "provider_turn_identity_mismatch")
    })
  }

  const h = harness({ run: { ...harness().state.run, session_id: "session-foreign" } })
  await assert.rejects(h.start(), (error) => error instanceof ProviderTurnBindingError
    && error.code === "provider_turn_identity_mismatch")
})

test("rejects PID reuse or provider identity changes between capture samples", async (t) => {
  await t.test("start-time changed", async () => {
    const h = harness()
    const binding = await h.start()
    h.state.providerStartTime += 1
    await assert.rejects(binding.finish(), (error) => error instanceof ProviderTurnBindingError
      && ["provider_process_changed", "provider_turn_changed"].includes(error.code))
  })

  await t.test("executable bytes changed", async () => {
    const h = harness()
    const binding = await h.start()
    h.state.providerExecutable = Buffer.from("replacement provider executable")
    await assert.rejects(binding.finish(), (error) => error instanceof ProviderTurnBindingError
      && error.code === "provider_turn_changed")
  })

  await t.test("boot identity changed", async () => {
    const h = harness()
    const binding = await h.start()
    h.state.bootId = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
    await assert.rejects(binding.finish(), (error) => error instanceof ProviderTurnBindingError
      && error.code === "provider_turn_changed")
  })
})

test("rejects deleted, exited, or unreadable provider processes", async (t) => {
  await t.test("deleted executable", async () => {
    const h = harness({ providerExecutableLink: "/usr/local/bin/codex (deleted)" })
    await assert.rejects(h.start(), (error) => error instanceof ProviderTurnBindingError
      && error.code === "provider_executable_deleted")
  })

  await t.test("exited process", async () => {
    const h = harness({ providerState: "Z" })
    await assert.rejects(h.start(), (error) => error instanceof ProviderTurnBindingError
      && error.code === "provider_process_exited")
  })

  await t.test("unreadable executable", async () => {
    const h = harness()
    const readlink = h.filesystem.readlink
    h.filesystem.readlink = async (path) => {
      if (path === `/proc/${PROVIDER_PID}/exe`) throw Object.assign(new Error("denied"), { code: "EACCES" })
      return readlink(path)
    }
    await assert.rejects(h.start(), (error) => error instanceof ProviderTurnBindingError
      && error.code === "provider_process_unreadable")
  })
})

test("rejects kernel identity and provider-run status changes during capture", async (t) => {
  await t.test("kernel identity changed", async () => {
    const h = harness()
    const binding = await h.start()
    h.state.kernelId = "kernel-foreign"
    await assert.rejects(binding.finish(), (error) => error instanceof ProviderTurnBindingError
      && error.code === "provider_turn_changed")
  })

  await t.test("run ended", async () => {
    const h = harness()
    const binding = await h.start()
    h.state.run = { ...h.state.run, state: "Ended" }
    await assert.rejects(binding.finish(), (error) => error instanceof ProviderTurnBindingError
      && error.code === "provider_turn_identity_mismatch")
  })
})


test("MP-10 observes the provider turn through the configured production WebSocket client", async () => {
  const h = harness()
  delete h.processApi.env.CHARIOX_DAEMON_SOCKET
  h.processApi.env.CHARIOX_KERNEL_URL = "ws://127.0.0.1:43118"
  const endpoints = []
  const binding = await h.start({ clientFactory: (endpoint, options) => {
    endpoints.push(endpoint)
    assert.equal(endpoint, "ws://127.0.0.1:43118/")
    assert.deepEqual(options, {})
    return h.clientFactory(SOCKET_PATH)
  } })
  const proof = await binding.finish()
  assert.equal(proof.kernel_identity.transport, "kernel-public-api")
  assert.equal(endpoints.length, 2)
  assert.equal(JSON.stringify(proof).includes("fixture-secret"), false)
})


test("MP-10 authenticated TLS relay targets the exact product kernel", async () => {
  const h = harness()
  delete h.processApi.env.CHARIOX_DAEMON_SOCKET
  h.processApi.env.CHARIOX_KERNEL_URL = "wss://relay.example.test/runtime"
  h.processApi.env.CHARIOX_PARITY_PROJECT_SETUP_RELAY_TOKEN = "fixture-relay-secret"
  const binding = await h.start({ clientFactory: (endpoint, options) => {
    assert.equal(endpoint, "wss://relay.example.test/runtime")
    assert.deepEqual(options, { relayAuthToken: "fixture-relay-secret", targetDaemonId: KERNEL_ID })
    return h.clientFactory(SOCKET_PATH)
  } })
  const proof = await binding.finish()
  assert.equal(proof.kernel_identity.transport, "relay")
  assert.equal(JSON.stringify(proof).includes("fixture-relay-secret"), false)
  h.state.kernelId = "kernel-foreign"
  await assert.rejects(h.start({ clientFactory: () => h.clientFactory(SOCKET_PATH) }), error => error.code === "kernel_identity_invalid")
})

test("MP-10 rejects remote unauthenticated or credential-bearing endpoint locators", async () => {
  for (const endpoint of ["ws://example.test:43118", "wss://example.test", "ws://secret@127.0.0.1:43118", "ws://127.0.0.1:43118?token=secret", "ws://127.0.0.1:43118/foreign"]) {
    const h = harness()
    delete h.processApi.env.CHARIOX_DAEMON_SOCKET
    h.processApi.env.CHARIOX_KERNEL_URL = endpoint
    await assert.rejects(h.start())
    assert.equal(h.state.requests.length, 0)
  }
})

// MP-11 F7: process provenance uses observed procfs, not private launch fields.
test("MP-11 binds a credential-free public run without launch arguments", async () => {
  const h = harness()
  for (const field of ["process_label", "pty_program", "pty_args", "pty_env", "runtime_mcp_auth_token"]) delete h.state.run[field]
  const proof = await (await h.start()).finish()
  assert.equal(proof.process.pid, PROVIDER_PID)
  assert.equal(proof.process.command_line_sha256.startsWith("sha256:"), true)
  assert.equal(proof.process.launch_arguments_sha256, null)
})
