// Exercises publication materialization through a disposable real kernel. It
// never connects to the user's kernel, relay, provider accounts or Cloud.
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { once } from "node:events"
import { mkdir, mkdtemp, readdir, rm, stat } from "node:fs/promises"
import { createServer } from "node:net"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import { LocalIpcClient } from "../packages/kernel-client/dist/ipc.js"

const binary = resolve(process.argv[2] ?? "target/debug/chariox-kernel")
const replaceEphemeralHome = process.argv.includes("--replace-ephemeral-home")
const activationBarrier = process.argv.includes("--activation-barrier")
const reconfigureProfile = process.argv.includes("--reconfigure-profile")
const root = await mkdtemp(join(tmpdir(), "chariox-publication-resume-"))
const workspace = join(root, "workspace")
const ephemeralHome = join(root, "home")
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
      CHARIOX_HOME: ephemeralHome,
      ...(replaceEphemeralHome || activationBarrier ? { CHARIOX_PUBLICATION_CONTROL_STATE_DIR: join(root, "control") } : {}),
      CHARIOX_KERNEL_HOST: "127.0.0.1",
      CHARIOX_KERNEL_PORT: String(kernelPort),
      CHARIOX_MCP_PORT: String(kernelMcpPort),
      CHARIOX_DAEMON_ID: "kernel-publication-resume-drill",
      CHARIOX_MACHINE_ID: "machine-publication-resume-drill",
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
    } catch {
      await client.close().catch(() => {})
      await delay(100)
      continue
    }
    if (replaceEphemeralHome) await assertPrivateStateIsEphemeral()
    return
  }
  throw new Error("disposable kernel did not become ready")
}

async function assertPrivateStateIsEphemeral() {
  const retained = await readdir(join(root, "control"))
  for (const name of ["provider-accounts.json", "provider-accounts", "managed-context-transfers", "managed-context-outbound"]) {
    assert.ok(!retained.includes(name), `private state retained in publication control volume: ${name}`)
  }
  const privateRoot = join(ephemeralHome, "kernels", "kernel-publication-resume-drill")
  assert.ok((await stat(join(privateRoot, "provider-accounts.json"))).isFile(), "account registry must be rebuilt outside the retained volume")
  assert.ok((await stat(join(privateRoot, "managed-context-transfers"))).isDirectory(), "incoming context must remain ephemeral")
  assert.ok((await stat(join(privateRoot, "managed-context-outbound"))).isDirectory(), "outgoing context must remain ephemeral")
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
  try {
    const [, actualSignal] = await exited
    if (signal === "SIGTERM") {
      assert.notEqual(actualSignal, "SIGKILL", "disposable kernel could not shut down and release its state lease")
    }
  } finally { clearTimeout(deadline) }
}

async function send(kind, body, responseKind) {
  let timeout
  const response = await Promise.race([
    client.send({ [kind]: body }),
    new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error(`${kind} timed out`)), 15_000) }),
  ]).finally(() => clearTimeout(timeout))
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
    // Match the gateway's destination workspace resolution, not a raw agent
    // projection whose optional workspace_id can be absent.
    schedules: [], agents: [{ ...agent, workspace_id: workspace, worktree_id: workspace }],
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

  if (activationBarrier) {
    const replicaRequest = { ...request, runtime_key: "deployment-a:replica-1" }
    const activation = { publication_id: request.publication_id, runtime_keys: [request.runtime_key, replicaRequest.runtime_key] }
    await send("MaterializeWorkflowPublication", replicaRequest, "WorkflowPublicationMaterialized")
    await assert.rejects(() => send("MaterializeWorkflowPublication", {
      ...request, snapshot: { ...snapshot, captured_at_ms: 43 },
    }, "WorkflowPublicationMaterialized"), /publication|snapshot/i)
    await assert.rejects(() => send("ActivateWorkflowPublicationRuntime", activation, "WorkflowPublicationRuntimeActivated"), /prepar|activation/i)
    await send("MaterializeWorkflowPublication", request, "WorkflowPublicationMaterialized")
    const { attachment } = await send("AttachToSession", { session_id: first.session.id, client_id: "activation-drill", capability_level: "FullTerminal" }, "SessionAttached")
    const { schedule } = await send("CreateWorkflowSchedule", {
      session_id: first.session.id, workflow_ref: workflow.id, endpoint_ref: endpointResult.endpoint.id,
      trigger: { kind: "interval", every_seconds: 1 }, invocation_prompt: "activation barrier",
      overlap_policy: "queue", max_runs_configured: true, max_runs: 2,
    }, "WorkflowScheduleCreated")
    const current = async () => (await send("GetSessionState", { session_id: first.session.id }, "SessionState")).session
    await delay(1600)
    assert.deepEqual((await current()).workflow_schedules, [schedule], "schedule advanced before runtime activation")
    await assert.rejects(() => send("ActivateWorkflowPublicationRuntime", {
      ...activation, runtime_keys: [request.runtime_key],
    }, "WorkflowPublicationRuntimeActivated"), /prepar|activation/i)
    await send("ActivateWorkflowPublicationRuntime", activation, "WorkflowPublicationRuntimeActivated")
    // A live projection may show admission before its durable transaction has
    // completed. Provider output proves dispatch crossed that transaction;
    // crash that externally observed occurrence rather than an in-flight read.
    let providerObserved = false
    for (let i = 0; i < 50 && !providerObserved; i++) {
      const { records } = await send("PumpTerminalOutput", {
        session_id: first.session.id, attachment_id: attachment.id,
      }, "TerminalOutput")
      providerObserved = records.some((record) => record.kind === "provider_output"
        && Buffer.from(record.bytes).toString("utf8").includes("activation barrier"))
      if (!providerObserved) await delay(100)
    }
    assert.ok(providerObserved, "activation did not dispatch the scheduled prompt to the provider")
    const active = await current()
    assert.equal(active.workflow_runs.length, 1, "activation did not admit the first schedule occurrence")
    await stop("SIGKILL")
    await rm(ephemeralHome, { recursive: true, force: true })
    await start()
    await send("AttachToSession", { session_id: first.session.id, client_id: "activation-drill", capability_level: "FullTerminal" }, "SessionAttached")
    await delay(1600)
    const held = await current()
    assert.deepEqual(held.workflow_schedules, active.workflow_schedules, "restart consumed a schedule before reactivation")
    assert.deepEqual(held.workflow_runs.map((run) => run.id), active.workflow_runs.map((run) => run.id))
    await assert.rejects(() => send("ActivateWorkflowPublicationRuntime", activation, "WorkflowPublicationRuntimeActivated"), /prepar|activation/i)
    await send("MaterializeWorkflowPublication", request, "WorkflowPublicationMaterialized")
    await assert.rejects(() => send("ActivateWorkflowPublicationRuntime", activation, "WorkflowPublicationRuntimeActivated"), /prepar|activation/i)
    await send("MaterializeWorkflowPublication", replicaRequest, "WorkflowPublicationMaterialized")
    await assert.rejects(() => send("ActivateWorkflowPublicationRuntime", {
      ...activation, runtime_keys: ["wrong-runtime"],
    }, "WorkflowPublicationRuntimeActivated"), /prepar|activation/i)
    await delay(1100)
    assert.deepEqual((await current()).workflow_schedules, active.workflow_schedules)
    await send("ActivateWorkflowPublicationRuntime", activation, "WorkflowPublicationRuntimeActivated")
    console.log(JSON.stringify({ passed: true, startupActivationHeld: true, restartActivationHeld: true,
      exactPreparationRequired: true, failedPreparationInvalidated: true, completeReplicaSetRequired: true, unknownRuntimeRejected: true, firstOccurrencePreserved: true }))
  } else {
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
  if (replaceEphemeralHome) await rm(ephemeralHome, { recursive: true, force: true })
  await start()
  const restored = await send("MaterializeWorkflowPublication", request, "WorkflowPublicationMaterialized")
  assert.equal(restored.session.id, first.session.id, "kernel restart lost the serving publication session")
  assert.deepEqual(restored.agent_id_map, first.agent_id_map, "kernel restart lost the publication agent mapping")
  assert.deepEqual(restored.session.workflow_schedules, [pausedSchedule], "restart reinstalled captured schedules instead of current schedule state")
  assert.deepEqual(restored.session.workflow_queued_prompts, [queuedPrompt], "restart lost or duplicated the queued workflow invocation")

  let recoveryRequest = request
  if (reconfigureProfile) {
    assert.ok(replaceEphemeralHome, "profile reconfiguration drill requires retained control state")
    recoveryRequest = { ...request, snapshot: { ...snapshot, agents: snapshot.agents.map((agent) => ({
      ...agent, model: "terminal-echo-a", effort: "low", account_profile: "destination-account",
    })) } }
    const { DatabaseSync } = await import("node:sqlite")
    const database = new DatabaseSync(join(root, "control", "state.db"))
    database.exec(`CREATE TRIGGER fail_profile_reconfiguration BEFORE INSERT ON durable_state_events
      WHEN NEW.kind = 'workflow.publication.reconfigured'
      BEGIN SELECT RAISE(FAIL, 'injected profile reconfiguration failure'); END;`)
    await assert.rejects(() => send("MaterializeWorkflowPublication", recoveryRequest, "WorkflowPublicationMaterialized"),
      /injected profile reconfiguration failure/)
    database.exec("DROP TRIGGER fail_profile_reconfiguration")
    database.close()
    const unchanged = await send("GetSessionState", { session_id: first.session.id }, "SessionState")
    assert.ok(unchanged.session.agents.every((agent) => agent.account_profile !== "destination-account"),
      "failed durable write leaked a partial profile change")
    assert.deepEqual(unchanged.session.workflow_schedules, [pausedSchedule], "failed profile write changed the schedule")
    assert.deepEqual(unchanged.session.workflow_queued_prompts, [queuedPrompt], "failed profile write changed queued work")
    const changed = await send("MaterializeWorkflowPublication", recoveryRequest, "WorkflowPublicationMaterialized")
    assert.equal(changed.session.id, first.session.id, "profile change replaced the serving session")
    assert.deepEqual(changed.agent_id_map, first.agent_id_map, "profile change replaced the serving agents")
    assert.deepEqual(changed.session.workflow_schedules, [pausedSchedule], "profile change reset the schedule")
    assert.deepEqual(changed.session.workflow_queued_prompts, [queuedPrompt], "profile change lost queued work")
    assert.ok(changed.session.agents.every((agent) => agent.model === "terminal-echo-a"
      && agent.effort === "low" && agent.account_profile === "destination-account"), "profile change was ignored")
  }

  await stop("SIGKILL")
  if (replaceEphemeralHome) await rm(ephemeralHome, { recursive: true, force: true })
  await start()
  const recovered = await send("MaterializeWorkflowPublication", recoveryRequest, "WorkflowPublicationMaterialized")
  assert.equal(recovered.session.id, first.session.id, "abrupt kernel death lost the runtime binding")
  assert.deepEqual(recovered.session.workflow_schedules, [pausedSchedule], "abrupt kernel death lost schedule state")
  assert.deepEqual(recovered.session.workflow_queued_prompts, [queuedPrompt], "abrupt kernel death lost queue identity/attribution")
  if (reconfigureProfile) {
    assert.ok(recovered.session.agents.every((agent) => agent.model === "terminal-echo-a"
      && agent.effort === "low" && agent.account_profile === "destination-account"), "restart lost the accepted profile change")
  }

  const other = await send("MaterializeWorkflowPublication", {
    ...request, runtime_key: "deployment-a:replica-1",
  }, "WorkflowPublicationMaterialized")
  assert.notEqual(other.session.id, first.session.id, "independent replicas shared a runtime session")
  assert.notDeepEqual(other.agent_id_map, first.agent_id_map, "independent replicas shared agents")
  if (reconfigureProfile) {
    await send("ActivateWorkflowPublicationRuntime", {
      publication_id: request.publication_id,
      runtime_keys: [request.runtime_key, "deployment-a:replica-1"],
    }, "WorkflowPublicationRuntimeActivated")
    await assert.rejects(() => send("MaterializeWorkflowPublication", request, "WorkflowPublicationMaterialized"),
      /restart|profile|activation|MaterializeWorkflowPublication/i)
  }
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
  console.log(JSON.stringify({ passed: true, replacedEphemeralHome: replaceEphemeralHome, profileReconfiguration: reconfigureProfile, concurrentCreation: true, retry: true, restart: true, crashRecovery: true,
    scheduleStatePreserved: true, queuedInvocationPreserved: true, exclusiveWriter: true, independentReplicas: true, unkeyedIndependent: true,
    profileWriteAtomic: reconfigureProfile, conflictingPublicationRejected: true, conflictingSnapshotRejected: true, disabledNotResurrected: true }))
  }
} finally {
  await stop()
  await rm(root, { recursive: true, force: true })
}
