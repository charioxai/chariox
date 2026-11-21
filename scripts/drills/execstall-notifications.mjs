import assert from "node:assert/strict"
import { randomBytes } from "node:crypto"
import { join } from "node:path"
import { setTimeout as delay } from "node:timers/promises"
import {
  addWorkflowNodeRequest, createWorkflowEndpointRequest, createWorkflowRequest,
  invokeWorkflowEndpointRequest, setWorkflowNodeCanCompleteRunRequest,
} from "../../packages/kernel-client/dist/ipc-workflow-requests.js"

export async function proveParkedPeerSubscription({ source, target, wait, logs, provider, expected }) {
  const model = provider === "dev-stub" ? "workflow-single-turn-node" : "opencode/big-pickle"
  const ready = kernel => wait(async () => {
    try { return (await kernel.send({ RelayStatus: null })).RelayStatus.status.connected }
    catch { return false }
  }, "kernel and relay to connect")
  await Promise.all([ready(source), ready(target)])
  const makeRoom = async (kernel, alias) => {
    const work = join(kernel.dir, "work")
    const result = await kernel.send({
      CreateSession: {
        workspace_id: work, worktree_id: work, alias,
        agent_defaults: { provider, model, effort: "low", permission_level: "yolo" },
      },
    })
    return { session_id: result.SessionCreated.session.id, agent_id: result.SessionCreated.agent.id }
  }
  const producer = await makeRoom(source, "revision-checks")
  const subscriber = await makeRoom(target, "review-triage")
  const id = producer.session_id
  const workflow = (await source.send(createWorkflowRequest(id, "revision-check"))).WorkflowCreated.workflow
  const node = (await source.send(addWorkflowNodeRequest(id, workflow.id, producer.agent_id))).WorkflowNodeAdded.node
  await source.send(setWorkflowNodeCanCompleteRunRequest(id, workflow.id, node.id, true))
  const endpoint = (await source.send(createWorkflowEndpointRequest(id, workflow.id, node.id, "entry"))).WorkflowEndpointCreated.endpoint
  const { source_id } = (await source.send({
    RegisterWorkflowNotificationSource: { session_id: id, workflow_ref: workflow.id, exit_node_id: node.id, enabled: true },
  })).WorkflowNotificationSourceRegistered.source
  await target.send({
    CreateAgentTrigger: {
      ...subscriber, source: { kind: "notification", source: { workflow: { source_id } } },
      busy_policy: "queue", prompt: "Report the review result",
    },
  })
  const subscriptionRow = async () => (await target.send({ ListNotificationSubscriptions: null }))
    .NotificationSubscriptions.subscriptions.find(row => row.subscription.source_id === source_id)
  const initial = await subscriptionRow()
  assert(initial)
  const { subscription_id } = initial.subscription
  assert.notEqual(initial.subscription.target_kernel_id, initial.subscription.source_kernel_id)
  await target.send({ SetAgentDeadline: { ...subscriber, deadline_ms: Date.now() + 500 } })
  await wait(async () => (await target.send({ GetAgentDeadline: subscriber })).AgentDeadline.deadline?.state === "reached",
    "the review agent's time limit")

  const passphrase = randomBytes(24).toString("hex")
  await source.send({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase } } })
  await source.send({ UnlockCredentialVault: { passphrase, duration: { kind: "until_kernel_stops" }, surface: "terminal" } })
  await source.send({ AttachToSession: { session_id: id, client_id: "execstall", capability_level: "FullTerminal" } })
  await source.send({
    LaunchProviderRun: { ...producer, adapter_key: provider, provider, account_profile: "default", model, variant: "low" },
  })
  const produce = async description => {
    const prompt = `Use validate_and_submit_workflow_run_output to submit this JSON string: {"summary":"${description}"}. Then finish.`
    const run = (await source.send(invokeWorkflowEndpointRequest(id, workflow.id, endpoint.id, prompt)))
      .WorkflowRunInvoked.workflow_run
    await wait(async () => {
      const current = (await source.send({ GetWorkflowRun: { session_id: id, workflow_run_ref: run.id } })).WorkflowRun.workflow_run
      assert(!["Failed", "Stopped"].includes(current.status), `The source workflow stopped with status ${current.status}`)
      return current.status === "Completed"
    }, "the workflow to complete", 150_000)
    return run.id
  }
  const occurrences = [await produce("first review")]
  const matchingLogs = message => logs(target.dir).filter(row => row.subscription_id === subscription_id && row.message === message)
  const refused = () => matchingLogs("an accepted notification could not be queued")
  const parked = () => matchingLogs("notification subscription parked")
  if (expected === "red") {
    await wait(() => refused().length >= 2, "the subscription to repeat the deadline error")
    return { provider, occurrences, repeatedRefusals: refused().length }
  }
  await wait(async () => (await subscriptionRow())?.paused && parked().length === 1, "the subscription to park")
  const trace = async () => (await target.send({ ReadNotificationTrace: { subscription_id } })).NotificationTrace.lines
  const reason = (await trace()).find(line => line.detail?.includes("time limit ended"))
  assert(reason, "The person's trace says why delivery stopped")

  await target.restart()
  await ready(target)
  const restored = await subscriptionRow()
  assert(restored?.paused, "The parked state survives a kernel restart")
  assert((await trace()).some(line => line.detail === reason.detail), "The reason survives a kernel restart")
  occurrences.push(await produce("second review"))
  await delay(6_000)
  assert.equal(refused().length, 0)
  assert.equal(parked().length, 1)
  await target.send({ RemoveNotificationSubscription: { subscription_id } })
  assert.equal(await subscriptionRow(), undefined, "The owner can remove the parked subscription")
  return { provider, occurrences, subscription_id, reason: reason.detail, parkLogs: parked().length, refused: refused().length }
}
