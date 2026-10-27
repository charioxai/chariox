import assert from "node:assert/strict"
import test from "node:test"
import { chmod, mkdir, mkdtemp, readFile, realpath, rm, stat, symlink } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { capturePath1FreshRelayObservation, parsePath1FreshRelayCaptureArgs } from "./path1-fresh-relay-capture.mjs"
import { validateCaptureOutput, writeCaptureOutput } from "./path1-provider-rebuild-capture.mjs"

const REQUEST = { QueryFreshRemoteMachineKernels: { machine_ref: "builder-west" } }

function fixture() {
  const now = Date.now()
  const payload = {
    machine_ref: "machine-resolved-1",
    query_started_at_ms: now - 40,
    query_completed_at_ms: now - 10,
    kernels: [{
      kernel_id: "kernel-1", machine_id: "machine-resolved-1", machine_alias: "builder-west",
      relay_alias: null, kernel_alias: "worker-blue", available_providers: ["LEAK_PROVIDER"],
      provider_accounts: [{ token: "LEAK_ACCOUNT" }], capabilities: ["LEAK_CAPABILITY"],
      accepting_remote_leases: true, leased_agent_count: 2, local_session_count: 3,
      public_key: "LEAK_PUBLIC_KEY",
    }],
  }
  const requests = []
  return {
    payload,
    requests,
    client: { async send(request) { requests.push(request); return { FreshRemoteMachineKernelsObserved: payload } } },
  }
}

test("captures exact fresh request and only scoped identity allowlist", async () => {
  const input = fixture()
  const capture = await capturePath1FreshRelayObservation({ client: input.client, machineRef: "builder-west" })
  assert.deepEqual(input.requests, [REQUEST])
  assert.equal(capture.minimumProtocolVersion, 346)
  assert.deepEqual(capture.scope, {
    requestedMachineRef: "builder-west", resolvedMachineRef: "machine-resolved-1",
    queryStartedAtMs: input.payload.query_started_at_ms, queryCompletedAtMs: input.payload.query_completed_at_ms,
  })
  assert.deepEqual(capture.kernels, [{
    kernelId: "kernel-1", machineId: "machine-resolved-1", machineAlias: "builder-west",
    relayAlias: null, kernelAlias: "worker-blue",
  }])
  assert.match(capture.limitation, /does not prove historical heartbeat-ID absence or full MP-10 acceptance/)
  for (const secret of ["LEAK_PROVIDER", "LEAK_ACCOUNT", "LEAK_CAPABILITY", "LEAK_PUBLIC_KEY"]) {
    assert.ok(!JSON.stringify(capture).includes(secret))
  }
  assert.equal(capture.verdict, undefined)
})

test("accepts the exact versioned protocol response shape and rejects cached projection fallback", async () => {
  const input = fixture()
  input.client.send = async (request) => {
    input.requests.push(request)
    return { RemoteMachineKernelsListed: input.payload }
  }
  await assert.rejects(capturePath1FreshRelayObservation({ client: input.client, machineRef: "builder-west" }), /fresh relay observation response/)
  assert.deepEqual(input.requests, [REQUEST])
})

test("rejects a resolved machine or registration outside the requested fresh scope", async () => {
  const wrongMachine = fixture()
  wrongMachine.payload.machine_ref = "machine-other"
  wrongMachine.payload.kernels[0].machine_id = "machine-resolved-1"
  wrongMachine.payload.kernels[0].machine_alias = null
  await assert.rejects(capturePath1FreshRelayObservation({ client: wrongMachine.client, machineRef: "builder-west" }), /outside the resolved machine scope/)

  const wrongKernel = fixture()
  wrongKernel.payload.kernels[0].machine_id = "machine-other"
  wrongKernel.payload.kernels[0].machine_alias = null
  await assert.rejects(capturePath1FreshRelayObservation({ client: wrongKernel.client, machineRef: "builder-west" }), /outside the resolved machine scope/)
})

test("rejects malformed, future, stale, or reversed query timestamps", async () => {
  for (const [name, mutate] of [
    ["reversed", (p) => { p.query_started_at_ms = p.query_completed_at_ms + 1 }],
    ["future", (p) => { p.query_completed_at_ms = Date.now() + 60_000 }],
    ["stale", (p) => { p.query_started_at_ms = Date.now() - 60_000; p.query_completed_at_ms = Date.now() - 59_000 }],
    ["malformed", (p) => { p.query_completed_at_ms = "now" }],
  ]) {
    const input = fixture()
    mutate(input.payload)
    await assert.rejects(capturePath1FreshRelayObservation({ client: input.client, machineRef: "builder-west" }), undefined, name)
  }
})

test("sanitizes relay and wrong-request-id errors and reports the unsupported protocol minimum", async () => {
  const input = fixture()
  input.client.send = async () => { throw new Error("opaque error token=do-not-retain") }
  await assert.rejects(capturePath1FreshRelayObservation({ client: input.client, machineRef: "builder-west" }), (error) => {
    assert.equal(error.message, "fresh relay observation request failed")
    assert.ok(!error.message.includes("do-not-retain"))
    return true
  })
  input.client.send = async () => { throw new Error("response request_id mismatch: opaque-request-id") }
  await assert.rejects(capturePath1FreshRelayObservation({ client: input.client, machineRef: "builder-west" }), (error) => {
    assert.equal(error.message, "fresh relay observation request failed")
    assert.ok(!error.message.includes("opaque-request-id"))
    return true
  })
  input.client.send = async () => { throw new Error("unknown variant `QueryFreshRemoteMachineKernels`") }
  await assert.rejects(capturePath1FreshRelayObservation({ client: input.client, machineRef: "builder-west" }), /protocol 346 or newer/)
})

test("a bounded timeout fails without a second cached request", async () => {
  const input = fixture()
  input.client.send = (request) => { input.requests.push(request); return new Promise(() => {}) }
  await assert.rejects(capturePath1FreshRelayObservation({ client: input.client, machineRef: "builder-west", timeoutMs: 15 }), /timed out/)
  assert.deepEqual(input.requests, [REQUEST])
})

test("parser accepts only the loopback kernel, machine scope and external output flags", () => {
  assert.deepEqual(parsePath1FreshRelayCaptureArgs([
    "--kernel", "ws://127.0.0.1:43118", "--machine", "builder-west", "--output", "/tmp/fresh.json",
  ]), { kernelEndpoint: "ws://127.0.0.1:43118", machineRef: "builder-west", outputPath: "/tmp/fresh.json" })
  for (const args of [
    ["--kernel", "ws://kernel.example:43118", "--machine", "builder-west", "--output", "/tmp/fresh.json"],
    ["--kernel", "ws://127.0.0.1:43118", "--machine", "builder-west", "--output", "/tmp/fresh.json", "--accept", "true"],
    ["--kernel", "ws://127.0.0.1:43118", "--kernel", "ws://127.0.0.1:43118", "--machine", "builder-west"],
  ]) assert.throws(() => parsePath1FreshRelayCaptureArgs(args))
})

test("external output helper writes mode 0600 with write-new semantics and blocks symlinked paths", async (t) => {
  const root = await realpath(await mkdtemp(join(tmpdir(), "chariox-fresh-relay-capture-")))
  t.after(() => rm(root, { recursive: true, force: true }))
  const repository = join(root, "repo")
  const evidence = join(root, "evidence")
  await mkdir(repository)
  await mkdir(evidence)
  const alias = join(evidence, "repository-alias")
  await symlink(repository, alias)
  await assert.rejects(validateCaptureOutput(join(repository, "capture.json"), repository), /outside the repository/)
  await assert.rejects(validateCaptureOutput(join(alias, "capture.json"), repository), /symlink/)

  const output = join(evidence, "capture.json")
  const capture = await capturePath1FreshRelayObservation({ ...fixture(), machineRef: "builder-west" })
  await writeCaptureOutput(output, capture, repository)
  assert.deepEqual(JSON.parse(await readFile(output, "utf8")), capture)
  assert.equal((await stat(output)).mode & 0o777, 0o600)
  await chmod(output, 0o600)
  await assert.rejects(writeCaptureOutput(output, capture, repository), /new file/)
})
