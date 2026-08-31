// Exercises publication materialization through a disposable real kernel. It
// never connects to the user's kernel, relay, provider accounts or Cloud.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { once } from "node:events"
import { mkdir, mkdtemp, rm } from "node:fs/promises"
import { createServer } from "node:net"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../packages/kernel-client/dist/ipc.js"

const binary = resolve(process.argv[2] ?? "target/debug/chariox-kernel")
const root = await mkdtemp(join(tmpdir(), "chariox-publication-resume-"))
const workspace = join(root, "workspace")
await mkdir(workspace)
async function reservePorts() {
  const reservations = [createServer(), createServer()]
  await Promise.all(reservations.map(async (server) => {
    server.listen(0, "127.0.0.1")
    await once(server, "listening")
  }))
  const ports = reservations.map((server) => server.address().port)
  await Promise.all(reservations.map((server) => new Promise((resolve) => server.close(resolve))))
  return ports
}
const [port, mcpPort] = await reservePorts()
let child
let client
let startupError = ""

function kernelEnv(kernelPort, kernelMcpPort) {
  return {
      PATH: process.env.PATH,
      HOME: process.env.HOME,
      USER: process.env.USER,
      LANG: "en_US.UTF-8",
      CHARIOX_HOME: join(root, "home"),
      CHARIOX_KERNEL_HOST: "127.0.0.1",
      CHARIOX_KERNEL_PORT: String(kernelPort),
      CHARIOX_MCP_PORT: String(kernelMcpPort),
      CHARIOX_DAEMON_ID: "kernel-publication-resume-drill",
      CHARIOX_PROVIDER_DEV_STUB: "1",
  }
}

async function start() {
  child = spawn(binary, [], {
    cwd: workspace,
    env: kernelEnv(port, mcpPort),
    stdio: ["ignore", "ignore", "pipe"],
  })
  child.stderr.on("data", (chunk) => { startupError = (startupError + chunk).slice(-4_096) })
  let failure
  child.on("error", (error) => { failure = error })
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (failure) throw failure
    assert.equal(child.exitCode, null, `disposable kernel exited during startup: ${startupError}`)
    client = new LocalIpcClient(`ws://127.0.0.1:${port}/kernel`, {
      controlRequestRetryDeadlineMs: 500,
      controlResponseStallMs: 500,
    })
    try {
      await client.send({ ListSessions: {} })
      return
    } catch {
      await client.close().catch(() => {})
      await delay(100)
    }
  }
  throw new Error("disposable kernel did not become ready")
}

async function assertWriterFenced() {
  const [otherPort, otherMcpPort] = await reservePorts()
  const contender = spawn(binary, [], {
    cwd: workspace, env: kernelEnv(otherPort, otherMcpPort), stdio: ["ignore", "ignore", "pipe"],
  })
  let errorText = ""
  contender.stderr.on("data", (chunk) => { errorText = (errorText + chunk).slice(-4096) })
  const exited = once(contender, "exit")
  try {
    const result = await Promise.race([exited, delay(3000).then(() => null)])
    assert.ok(result, "a second kernel may own the same durable state and scheduler")
    assert.notEqual(result[0], 0, "competing kernel succeeded")
    assert.match(errorText, /durable state.*already owned/i)
  } finally {
    if (contender.exitCode === null && contender.signalCode === null) {
      contender.kill("SIGKILL")
      await exited
    }
  }
}

async function stop(signal = "SIGTERM") {
  await client?.close().catch(() => {})
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  const exited = once(child, "exit")
  child.kill(signal)
  const deadline = setTimeout(() => child.kill("SIGKILL"), 5_000)
  try { await exited } finally { clearTimeout(deadline) }
}

async function send(kind, body, responseKind) {
  const response = await client.send({ [kind]: body })
  assert.ok(response[responseKind], `${kind} did not return ${responseKind}`)
  return response[responseKind]
}

try {
  await start()
  const { session } = await send("CreateSession", {
    workspace_id: workspace, worktree_id: workspace,
  }, "SessionCreated")
  const { agent } = await send("SpawnAgent", {
    session_id: session.id, provider: "dev-stub", model: "default", alias: "worker",
  }, "AgentSpawned")
  const { workflow } = await send("CreateWorkflow", { session_id: session.id, alias: "resume" }, "WorkflowCreated")
  const { node } = await send("AddWorkflowNode", {
    session_id: session.id, workflow_ref: workflow.id, agent_id: agent.id,
  }, "WorkflowNodeAdded")
  const endpointResult = await send("CreateWorkflowEndpoint", {
    session_id: session.id, workflow_ref: workflow.id, entry_node_id: node.id, alias: "entry",
  }, "WorkflowEndpointCreated")
  const source = await send("GetSessionState", { session_id: session.id }, "SessionState")
  const snapshot = {
    schema_version: 1, captured_at_ms: 42,
    source_session: { id: session.id, workspace_id: workspace, worktree_id: workspace },
    workflow: endpointResult.workflow, endpoint: endpointResult.endpoint,
    queues: source.session.workflow_prompt_queues,
    schedules: [], agents: [agent],
  }
  const request = { publication_id: "resume-publication", runtime_key: "deployment-a:replica-0", snapshot }
  const initial = await Promise.all(Array.from({ length: 4 }, () =>
    send("MaterializeWorkflowPublication", request, "WorkflowPublicationMaterialized")))
  const first = initial[0]
  assert.ok(initial.every((result) => result.session.id === first.session.id), "concurrent first requests created duplicate runtime sessions")
  const retry = await send("MaterializeWorkflowPublication", request, "WorkflowPublicationMaterialized")
  assert.equal(retry.session.id, first.session.id, "retry created a fresh publication runtime session")
  assert.deepEqual(retry.agent_id_map, first.agent_id_map, "retry replaced publication agents")
  await assertWriterFenced()

  const { schedule } = await send("CreateWorkflowSchedule", {
    session_id: first.session.id, workflow_ref: workflow.id, endpoint_ref: endpointResult.endpoint.id,
    trigger: { kind: "interval", every_seconds: 3600 }, invocation_prompt: "resume schedule",
    overlap_policy: "queue", max_runs_configured: true, max_runs: 2,
  }, "WorkflowScheduleCreated")
  const { schedule: pausedSchedule } = await send("SetWorkflowScheduleEnabled", {
    session_id: first.session.id, schedule_ref: schedule.id, enabled: false,
  }, "WorkflowScheduleUpdated")
  await send("UpdateWorkflowPromptQueue", {
    session_id: first.session.id, workflow_ref: workflow.id, queue_ref: "default", enabled: false,
  }, "WorkflowPromptQueueUpdated")
  const { queued_prompt: queuedPrompt } = await send("InvokeWorkflowEndpoint", {
    session_id: first.session.id, workflow_ref: workflow.id, endpoint_ref: endpointResult.endpoint.id,
    queue_ref: "default", prompt: "queued before publication restart",
  }, "WorkflowPromptEnqueued")

  await stop()
  await start()
  const restored = await send("MaterializeWorkflowPublication", request, "WorkflowPublicationMaterialized")
  assert.equal(restored.session.id, first.session.id, "kernel restart lost the serving publication session")
  assert.deepEqual(restored.agent_id_map, first.agent_id_map, "kernel restart lost the publication agent mapping")
  assert.deepEqual(restored.session.workflow_schedules, [pausedSchedule], "restart reinstalled captured schedules instead of current schedule state")
  assert.deepEqual(restored.session.workflow_queued_prompts, [queuedPrompt], "restart lost or duplicated the queued workflow invocation")

  await stop("SIGKILL")
  await start()
  const recovered = await send("MaterializeWorkflowPublication", request, "WorkflowPublicationMaterialized")
  assert.equal(recovered.session.id, first.session.id, "abrupt kernel death lost the runtime binding")
  assert.deepEqual(recovered.session.workflow_schedules, [pausedSchedule], "abrupt kernel death lost schedule state")
  assert.deepEqual(recovered.session.workflow_queued_prompts, [queuedPrompt], "abrupt kernel death lost queue identity/attribution")

  const other = await send("MaterializeWorkflowPublication", {
    ...request, runtime_key: "deployment-a:replica-1",
  }, "WorkflowPublicationMaterialized")
  assert.notEqual(other.session.id, first.session.id, "independent replicas shared a runtime session")
  assert.notDeepEqual(other.agent_id_map, first.agent_id_map, "independent replicas shared agents")
  await assert.rejects(() => send("MaterializeWorkflowPublication", {
    ...request, publication_id: "different-publication",
  }, "WorkflowPublicationMaterialized"), /runtime key|publication|MaterializeWorkflowPublication/i)
  await assert.rejects(() => send("MaterializeWorkflowPublication", {
    ...request, snapshot: { ...snapshot, captured_at_ms: 43 },
  }, "WorkflowPublicationMaterialized"), /runtime key|publication|MaterializeWorkflowPublication/i)
  const { runtime_key: _key, ...unkeyed } = request
  const independentA = await send("MaterializeWorkflowPublication", unkeyed, "WorkflowPublicationMaterialized")
  const independentB = await send("MaterializeWorkflowPublication", unkeyed, "WorkflowPublicationMaterialized")
  assert.notEqual(independentA.session.id, independentB.session.id, "unkeyed requests stopped creating independent instances")
  await send("DisableWorkflowPublication", {
    session_id: first.session.id, publication_ref: request.publication_id,
  }, "WorkflowPublicationDisabled")
  await assert.rejects(() => send("MaterializeWorkflowPublication", request, "WorkflowPublicationMaterialized"),
    /no longer resumable|publication|MaterializeWorkflowPublication/i)
  console.log(JSON.stringify({ passed: true, concurrentCreation: true, retry: true, restart: true, crashRecovery: true,
    scheduleStatePreserved: true, queuedInvocationPreserved: true, exclusiveWriter: true, independentReplicas: true, unkeyedIndependent: true,
    conflictingPublicationRejected: true, conflictingSnapshotRejected: true, disabledNotResurrected: true }))
} finally {
  await stop()
  await rm(root, { recursive: true, force: true })
}
