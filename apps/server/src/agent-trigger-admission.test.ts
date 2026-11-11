import test from "node:test"
import assert from "node:assert/strict"
import {mkdtempSync, rmSync, writeFileSync} from "node:fs"
import {tmpdir} from "node:os"
import {join} from "node:path"
import {buildHostedAgentTriggerServer} from "./hosted-agent-trigger-server.js"
import {loadSelfHostedAdmissionConfig, remoteAdmission, selfHostedAdmission, type SelfHostedAdmissionConfig} from "./agent-trigger-admission.js"

const token = "t".repeat(40)
const config: SelfHostedAdmissionConfig = {
  relay: {url: "wss://relay.example/", token: "relay-secret"},
  deployments: [{id: "nightly", kernelId: "kernel-1", sessionId: "session-1", triggerId: "trigger-1", accessToken: token}],
}

function ingress() {
  const sent: unknown[] = [], bootstraps: unknown[] = []
  const app = buildHostedAgentTriggerServer({publicOrigin: "https://ingress.example", admission: selfHostedAdmission(config), connect: (async (kernel: string, bootstrap: (key: string) => Promise<unknown>) => {
    bootstraps.push(await bootstrap("a".repeat(64)))
    return {send: async (value: unknown) => {sent.push(value); return {AgentTriggerInvoked: {invocation: {status: "accepted", prompt_id: "prompt-1", outcome: "queued"}}}}, close: async () => {}}
  }) as never})
  return {app, sent, bootstraps}
}

test("self-hosted admission fires a trigger for the holder of its access token and hands the operator relay to the connection only", async () => {
  const {app, sent, bootstraps} = ingress()
  try {
    const fired = await app.inject({method: "POST", url: "/agent-triggers/nightly/", headers: {authorization: `Bearer ${token}`, "content-type": "text/plain"}, payload: "run the nightly check"})
    assert.equal(fired.statusCode, 202); assert.equal(fired.json().prompt_id, "prompt-1")
    const request = (sent[0] as {InvokeAgentTrigger: {session_id: string; trigger_id: string; payload: {prompt: string}}}).InvokeAgentTrigger
    assert.ok(request.session_id === "session-1" && request.trigger_id === "trigger-1" && request.payload.prompt === "run the nightly check")
    assert.deepEqual((bootstraps[0] as {target: unknown}).target, {daemonId: "kernel-1", kernelRef: "kernel-1"})
    assert.ok(!fired.body.includes("relay-secret") && !JSON.stringify(fired.headers).includes("relay-secret"), "the relay credential never reaches the caller")
  } finally {await app.close()}
})

test("self-hosted admission refuses a missing or wrong token and an unknown deployment before any dispatch", async () => {
  const {app, sent} = ingress()
  try {
    const cases: Array<[string, Record<string, string>, number]> = [
      ["nightly", {}, 401], ["nightly", {authorization: "Bearer wrong"}, 401], ["nightly", {authorization: `Basic ${token}`}, 401], ["nightly", {authorization: `Bearer ${token}x`}, 401],
      ["other", {authorization: `Bearer ${token}`}, 404],
    ]
    for (const [id, headers, status] of cases) assert.equal((await app.inject({method: "POST", url: `/agent-triggers/${id}/`, headers: {"content-type": "text/plain", ...headers}, payload: "go"})).statusCode, status, JSON.stringify([id, headers]))
    const html = await app.inject({url: "/agent-triggers/nightly/", headers: {accept: "text/html"}})
    assert.equal(html.statusCode, 401, "no sign-in page is configured, so a refusal is a plain 401")
    assert.deepEqual(sent, [])
  } finally {await app.close()}
})

test("self-hosted deployments file must be private, wss or loopback, with strong distinct tokens", () => {
  const dir = mkdtempSync(join(tmpdir(), "trigger-admission-"))
  try {
    const write = (value: unknown, mode = 0o600) => {const path = join(dir, `d${Math.random()}.json`); writeFileSync(path, JSON.stringify(value), {mode}); return path}
    assert.equal(loadSelfHostedAdmissionConfig(write(config)).deployments[0]!.id, "nightly")
    assert.equal(loadSelfHostedAdmissionConfig(write({...config, relay: {...config.relay, url: "ws://127.0.0.1:9000"}})).relay.url, "ws://127.0.0.1:9000/")
    assert.throws(() => loadSelfHostedAdmissionConfig(write(config, 0o644)), /make it private/)
    assert.throws(() => loadSelfHostedAdmissionConfig(write({...config, relay: {...config.relay, url: "ws://relay.example"}})), /wss:\/\//)
    assert.throws(() => loadSelfHostedAdmissionConfig(write({...config, deployments: [{...config.deployments[0], accessToken: "short"}]})), /accessToken/)
    assert.throws(() => loadSelfHostedAdmissionConfig(write({...config, deployments: [config.deployments[0], config.deployments[0]]})), /listed twice/)
    assert.throws(() => loadSelfHostedAdmissionConfig(write({...config, deployments: []})), /at least one deployment/)
  } finally {rmSync(dir, {recursive: true, force: true})}
})

test("a remote admission URL needs an {id} placeholder and HTTPS or loopback", () => {
  const publicOrigin = new URL("https://ingress.example")
  assert.throws(() => remoteAdmission({url: "https://admit.example/fixed", publicOrigin, forwardedHeaders: []}), /\{id\}/)
  assert.throws(() => remoteAdmission({url: "http://admit.example/{id}", publicOrigin, forwardedHeaders: []}), /HTTPS or a loopback/)
  remoteAdmission({url: "https://admit.example/{id}/admission", publicOrigin, forwardedHeaders: []})
  remoteAdmission({url: "http://127.0.0.1:3000/{id}/admission", publicOrigin, forwardedHeaders: []})
})
