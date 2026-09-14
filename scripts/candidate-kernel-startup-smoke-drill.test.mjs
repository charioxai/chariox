import assert from "node:assert/strict"
import test from "node:test"

import {
  assertCandidateProtocolOutput,
  assertNoRelayOrProviderEnvironment,
  candidateKernelEndpoint,
  makeCandidateKernelEnvironment,
  parseCandidateKernelSmokeArgs,
} from "./candidate-kernel-startup-smoke-drill-helpers.mjs"

test("candidate kernel smoke requires an explicit binary and expected protocol", () => {
  const parsed = parseCandidateKernelSmokeArgs([
    "--binary", "/opt/candidate/chariox-kernel",
    "--expected-protocol", "326",
    "--timeout-ms", "45000",
  ])
  assert.deepEqual(parsed, {
    help: false,
    binary: "/opt/candidate/chariox-kernel",
    expectedProtocol: 326,
    timeoutMs: 45000,
  })
  assert.throws(() => parseCandidateKernelSmokeArgs(["--binary", "/tmp/kernel"]), /expected-protocol is required/)
  assert.throws(() => parseCandidateKernelSmokeArgs(["--expected-protocol", "326"]), /binary is required/)
})

test("candidate protocol preflight accepts only the exact reported version", () => {
  assert.equal(assertCandidateProtocolOutput("326\n", 326), 326)
  assert.throws(() => assertCandidateProtocolOutput("325\n", 326), /expected 326, got 325/)
  assert.throws(() => assertCandidateProtocolOutput("", 326), /expected 326, got <empty>/)
})

test("candidate environment is loopback, task-owned, auth-file based, and relay/provider free", () => {
  const env = makeCandidateKernelEnvironment({
    rootDir: "/tmp/candidate-kernel-smoke/run-1",
    tokenFile: "/tmp/candidate-kernel-smoke/run-1/auth-token",
    runId: "run-1",
    ports: { kernelPort: 43181, mcpPort: 43182, openCodePort: 43183, codexPort: 43184 },
    baseEnv: { PATH: "/usr/bin", USER: "tester" },
  })
  assert.equal(env.HOME, "/tmp/candidate-kernel-smoke/run-1/home")
  assert.equal(env.CHARIOX_HOME, "/tmp/candidate-kernel-smoke/run-1/chariox-home")
  assert.equal(env.CHARIOX_KERNEL_HOST, "127.0.0.1")
  assert.equal(env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE, "/tmp/candidate-kernel-smoke/run-1/auth-token")
  assert.equal(env.CHARIOX_ACCEPT_REMOTE_LEASES, "0")
  assertNoRelayOrProviderEnvironment(env)
  assert.equal(candidateKernelEndpoint(43181), "ws://127.0.0.1:43181")
  assert.throws(() => candidateKernelEndpoint(0), /port is invalid/)
})
