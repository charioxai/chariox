// MP-11: admitted publication callers may observe only their own private runs.
import type { TestContext } from "node:test"
import { createHmac } from "node:crypto"
import { LocalIpcClient } from "@chariox/kernel-client/ipc"
import { assert, baseConfig, buildServer, test } from "../index.test-support.js"
import {
  configurePublicationCallerClaimsRuntimeForTests,
  publicationInvocationCaller,
  type VerifiedPublicationCallerClaims,
} from "../publication-caller-claims.js"
import { publicationInvocationEnvelope } from "../kernel-publication-client.js"
import type { WorkflowPublicationConfig, WorkflowRun } from "../publication-types.js"

const fixtureSecret = "mp11-synthetic-publication-claims-0123456789"
const now = new Date("2026-10-05T14:17:00Z")
const seconds = now.getTime() / 1000
let sequence = 0
const config: WorkflowPublicationConfig = {
  ...baseConfig, transport: "human_http", route: "/", mode: "async",
  sync_timeout_ms: 20, poll_ms: 1,
  trace_exposure: { nodes: { node: ["thinking", "output_summary"] } },
}
function caller(subject = "user:A"): VerifiedPublicationCallerClaims {
  return { subject, accountId: "account", deploymentId: "deployment", environmentId: "environment",
    invocationId: "invocation-A", nonce: "fixture", roles: ["public"], issuedAt: now, expiresAt: new Date(now.getTime() + 60000) }
}
function headers(subject: string, account = "account") {
  const invocationId = `read-${++sequence}`
  const header = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url")
  const payload = Buffer.from(JSON.stringify({ iss: "chariox-cloud", aud: "deployment", sub: subject,
    org: account, deployment_id: "deployment", environment_id: "environment", roles: ["public"],
    invocation_id: invocationId, nonce: invocationId, iat: seconds, exp: seconds + 60 })).toString("base64url")
  const unsigned = `${header}.${payload}`
  return { "x-chariox-caller-claims": `${unsigned}.${createHmac("sha256", fixtureSecret).update(unsigned).digest("base64url")}`,
    "x-chariox-invocation-id": invocationId }
}
function privateRun(): WorkflowRun {
  return {
    id: "run-A", workflow_id: "workflow-1", endpoint_id: "endpoint-1", status: "Completed",
    publication_invocation: publicationInvocationEnvelope(config, {
      publication_id: config.publication_id, request_id: "invocation-A", mode: "async",
      input: { prompt: "private-A-prompt" }, caller: publicationInvocationCaller(caller(), {}),
    }),
    final_output: { message: "private-A-output" },
    intermediate_outputs: [{ id: "partial-A", output: { message: "private-A-partial" }, valid: true }],
    node_runs: [{ id: "node-run-A", node_id: "node", agent_id: "agent", status: "Completed",
      summary: "private-A-summary", thinking_traces: [{ id: "trace-A", message: "private-A-thinking", timestamp_ms: 1 }] }],
  }
}
const paths = [
  "/.well-known/chariox/publication/status",
  "/.well-known/chariox/publication/viewer/invocations/invocation-A",
  "/.well-known/chariox/publication/runs/run-A/events",
  "/.well-known/chariox/publication/invocations/invocation-A/events",
]
function mockKernel(t: TestContext, getRun: () => WorkflowRun) {
  t.mock.method(LocalIpcClient.prototype, "send", async (request: Record<string, unknown>) => {
    const kind = Object.keys(request)[0]
    const responses: Record<string, unknown> = {
      ListWorkflowRuns: { WorkflowRunsListed: { workflow_runs: [getRun()] } },
      GetWorkflowRun: { WorkflowRun: { workflow_run: getRun() } },
      AttachToSession: { SessionAttached: { attachment: { id: "test-attachment" } } },
      ListWorkflowWatchdogs: { WorkflowWatchdogsListed: { watchdogs: [] } },
      ListQueuedWorkflowPrompts: { QueuedWorkflowPromptsListed: { queued_prompts: [] } },
      ListWorkflowPromptQueues: { WorkflowPromptQueuesListed: { queues: [] } },
    }
    return responses[kind ?? ""] ?? {}
  })
}
for (const path of paths) {
  test(`MP-11 caller B cannot read caller A via ${path}`, async (t) => {
    configurePublicationCallerClaimsRuntimeForTests({ deploymentId: "deployment", environmentId: "environment",
      secret: fixtureSecret, now: () => now })
    t.after(() => configurePublicationCallerClaimsRuntimeForTests(undefined))
    mockKernel(t, privateRun)
    const { app } = buildServer(config, { getProviderReadiness: async () => [] })
    t.after(() => app.close())
    const response = await app.inject({ method: "GET", url: path, headers: headers("user:B") })
    assert.doesNotMatch(response.body, /private-A-|trace-A|node-run-A|partial-A/)
    const own = await app.inject({ method: "GET", url: path, headers: headers("user:A") })
    assert.equal(own.statusCode, 200)
    assert.match(own.body, /private-A-output/)
    if (path.endsWith("/events") || path.endsWith("/status")) assert.match(own.body, /private-A-thinking/)
  })
}

test("MP-11 anonymous public runs are shared but authenticated runs stay private", async (t) => {
  configurePublicationCallerClaimsRuntimeForTests(null)
  t.after(() => configurePublicationCallerClaimsRuntimeForTests(undefined))
  let run = privateRun()
  mockKernel(t, () => run)
  const { app } = buildServer(config, { getProviderReadiness: async () => [] })
  t.after(() => app.close())
  for (const url of paths) {
    const privateResponse = await app.inject({ method: "GET", url })
    assert.doesNotMatch(privateResponse.body, /private-A-/)
  }
  run = { ...run, publication_invocation: { ...run.publication_invocation!, caller: { type: "anonymous" } } }
  for (const url of paths) {
    const publicResponse = await app.inject({ method: "GET", url })
    assert.match(publicResponse.body, /private-A-output/)
  }
})

test("MP-11 same subject in another account and foreign publication runs stay private", async (t) => {
  configurePublicationCallerClaimsRuntimeForTests({ deploymentId: "deployment", environmentId: "environment",
    secret: fixtureSecret, now: () => now })
  t.after(() => configurePublicationCallerClaimsRuntimeForTests(undefined))
  let run = privateRun()
  mockKernel(t, () => run)
  const { app } = buildServer(config, { getProviderReadiness: async () => [] })
  t.after(() => app.close())
  for (const url of paths) {
    const response = await app.inject({ method: "GET", url, headers: headers("user:A", "other-account") })
    assert.doesNotMatch(response.body, /private-A-/)
  }
  run = { ...run, publication_invocation: { ...run.publication_invocation!, publication_id: "other-publication" } }
  for (const url of paths) {
    const response = await app.inject({ method: "GET", url, headers: headers("user:A") })
    assert.doesNotMatch(response.body, /private-A-/)
  }
})

for (const invocationLookup of [false, true]) {
  test(`MP-11 ${invocationLookup ? 'invocation' : 'run'} SSE rechecks caller ownership on later fetches`, async (t) => {
    configurePublicationCallerClaimsRuntimeForTests({ deploymentId: "deployment", environmentId: "environment",
      secret: fixtureSecret, now: () => now })
    t.after(() => configurePublicationCallerClaimsRuntimeForTests(undefined))
    const foreign = privateRun()
    const owned: WorkflowRun = { ...foreign, status: "Running", final_output: undefined,
      intermediate_outputs: [], node_runs: [], publication_invocation: {
        ...foreign.publication_invocation!, input: {}, caller: publicationInvocationCaller(caller("user:B"), {}),
      } }
    let reads = 0
    t.mock.method(LocalIpcClient.prototype, "send", async (request: Record<string, unknown>) => {
      if ("ListWorkflowRuns" in request) return { WorkflowRunsListed: { workflow_runs: [owned] } }
      if ("GetWorkflowRun" in request) {
        reads += 1
        return { WorkflowRun: { workflow_run: invocationLookup || reads > 1 ? foreign : owned } }
      }
      return {}
    })
    const { app } = buildServer(config, { getProviderReadiness: async () => [] })
    t.after(() => app.close())
    const url = invocationLookup ? paths[3]! : paths[2]!
    const response = await app.inject({ method: "GET", url, headers: headers("user:B") })
    assert.match(response.body, /event: status/)
    assert.match(response.body, /workflow run not found/)
    assert.doesNotMatch(response.body, /private-A-|trace-A|node-run-A|partial-A/)
  })
}

test("MP-11 authenticated status admission also applies without an HTTP ingress hook", async (t) => {
  configurePublicationCallerClaimsRuntimeForTests({ deploymentId: "deployment", environmentId: "environment",
    secret: fixtureSecret, now: () => now })
  t.after(() => configurePublicationCallerClaimsRuntimeForTests(undefined))
  mockKernel(t, privateRun)
  for (const kind of [undefined, "event_based"] as const) {
    const { app } = buildServer({ ...config, ...(kind ? { kind, transport: "event_based" } : {}) }, {
      getProviderReadiness: async () => [],
    })
    try {
      const missingClaims = await app.inject({ method: "GET", url: paths[0]! })
      assert.equal(missingClaims.statusCode, 401)
      assert.doesNotMatch(missingClaims.body, /private-A-/)
      const other = await app.inject({ method: "GET", url: paths[0]!, headers: headers("user:B") })
      assert.equal(other.statusCode, 200)
      assert.doesNotMatch(other.body, /private-A-/)
      const own = await app.inject({ method: "GET", url: paths[0]!, headers: headers("user:A") })
      assert.match(own.body, /private-A-output/)
    } finally {
      await app.close()
    }
  }
})
