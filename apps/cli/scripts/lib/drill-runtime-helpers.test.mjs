import assert from "node:assert/strict"
import { mkdtemp, mkdir, rm, utimes, writeFile } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  findMatchingProcessIdsFromPsOutput,
  formatDrillCommandLine,
  providerAuthFailureFromTerminalText,
  resolveBuiltBinary,
  resolveBuiltBinarySync,
  runLogged,
  waitForCondition,
  waitForTcpPort,
  withDevStubProviderInventory,
} from "./drill-runtime-helpers.mjs"
import { cleanupHostedCloudIdentity } from "./live-hosted-cloud-relay-drill-helpers.mjs"

test("hosted Cloud cleanup offlines kernels, revokes identities, and logs out without exposing the session", async () => {
  const calls = []
  const logs = []
  await cleanupHostedCloudIdentity({
    profile: {
      accountId: "account-1",
      accountSlug: "hosted-cleanup",
      realmId: "realm-1",
    },
    cloudSessionToken: "session-1",
    clientIds: ["client-1", "client-1"],
    machineIds: ["machine-1"],
    kernelPresences: [{ machineId: "machine-1", kernelId: "kernel-1" }],
    baseUrl: "https://cloud.example",
    post: async (url, body) => { calls.push({ url, body }) },
    logger: (event, details) => { logs.push({ event, details }) },
  })

  assert.deepEqual(calls.map((call) => call.url), [
    "https://cloud.example/kernels/presence",
    "https://cloud.example/clients/revoke",
    "https://cloud.example/machines/revoke",
    "https://cloud.example/auth/logout",
  ])
  assert.equal(calls[0].body.status, "OFFLINE")
  assert.equal(calls[3].body.sessionToken, "session-1")
  assert.deepEqual(logs, [{
    event: "cloud-identity-cleanup",
    details: {
      accountSlug: "hosted-cleanup",
      clients: ["client-1"],
      machines: ["machine-1"],
      kernels: ["kernel-1"],
      logout: true,
    },
  }])
  assert.doesNotMatch(JSON.stringify(logs), /session-1/)
})

test("hosted Cloud cleanup attempts revocation and logout after an earlier cleanup failure", async () => {
  const calls = []
  await assert.rejects(
    () => cleanupHostedCloudIdentity({
      profile: { accountId: "account-1", accountSlug: "hosted-cleanup", realmId: "realm-1" },
      cloudSessionToken: "session-1",
      clientIds: ["client-1"],
      machineIds: ["machine-1"],
      kernelPresences: [{ machineId: "machine-1", kernelId: "kernel-1" }],
      baseUrl: "https://cloud.example",
      post: async (url) => {
        calls.push(url)
        if (url.endsWith("/kernels/presence")) throw new Error("presence rejected")
      },
      logger: () => {},
    }),
    /hosted Cloud identity cleanup failed/,
  )
  assert.deepEqual(calls, [
    "https://cloud.example/kernels/presence",
    "https://cloud.example/clients/revoke",
    "https://cloud.example/machines/revoke",
    "https://cloud.example/auth/logout",
  ])
})

test("dev-stub drill inventory is enabled explicitly without mutating the source environment", () => {
  const source = { PATH: "/usr/bin", ARROBA_PROVIDER_DEV_STUB: "0" }
  const enabled = withDevStubProviderInventory(source)

  assert.deepEqual(enabled, { PATH: "/usr/bin", ARROBA_PROVIDER_DEV_STUB: "1" })
  assert.equal(source.ARROBA_PROVIDER_DEV_STUB, "0")
})

test("built binary resolution chooses the newest Cargo target candidate", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-binary-"))
  const manifestPath = path.join(root, "apps", "kernel", "Cargo.toml")
  const crateBinary = path.join(root, "apps", "kernel", "target", "debug", "arroba-kernel")
  const workspaceBinary = path.join(root, "target", "debug", "arroba-kernel")
  try {
    await mkdir(path.dirname(crateBinary), { recursive: true })
    await mkdir(path.dirname(workspaceBinary), { recursive: true })
    await writeFile(crateBinary, "stale")
    await writeFile(workspaceBinary, "current")
    await utimes(crateBinary, new Date(1_000), new Date(1_000))
    await utimes(workspaceBinary, new Date(2_000), new Date(2_000))

    assert.equal(resolveBuiltBinarySync(crateBinary, manifestPath, "arroba-kernel"), workspaceBinary)
    assert.equal(await resolveBuiltBinary(crateBinary, manifestPath, "arroba-kernel"), workspaceBinary)

    await utimes(crateBinary, new Date(3_000), new Date(3_000))
    assert.equal(resolveBuiltBinarySync(crateBinary, manifestPath, "arroba-kernel"), crateBinary)
    assert.equal(await resolveBuiltBinary(crateBinary, manifestPath, "arroba-kernel"), crateBinary)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("detects ANSI-rendered provider authentication failures", () => {
  assert.equal(
    providerAuthFailureFromTerminalText("\x1b[8BLogin\x1b[9Gexpired\x1b[17G \u00b7 Please run /login"),
    "Login expired",
  )
  assert.equal(
    providerAuthFailureFromTerminalText("\x1b[93C\x1b[35BNot\x1b[98Glogged\x1b[105Gin \u00b7 Run /login"),
    "Not logged in",
  )
  assert.equal(providerAuthFailureFromTerminalText("CLAUDECHARLIE"), null)
})

test("formatDrillCommandLine quotes replayable command diagnostics", () => {
  assert.equal(
    formatDrillCommandLine("node", ["apps/cli/scripts/drill-validation-suite.mjs", "--label", "Alice's local run"]),
    "node apps/cli/scripts/drill-validation-suite.mjs --label 'Alice'\\''s local run'",
  )
})

test("runLogged failure diagnostics use formatted command lines", async () => {
  await assert.rejects(
    () => runLogged(process.execPath, ["-e", "process.exit(7)", "arg with space"]),
    /node.* -e 'process\.exit\(7\)' 'arg with space' exited with 7/,
  )
})

test("waitForCondition returns the first ready observation", async () => {
  let count = 0
  const observed = await waitForCondition({
    label: "counter",
    timeoutMs: 100,
    pollMs: 1,
    observe: async () => ({ count: ++count }),
    isReady: (value) => value.count === 2,
  })

  assert.deepEqual(observed, { count: 2 })
})

test("waitForCondition reports the last observation on timeout", async () => {
  await assert.rejects(
    () => waitForCondition({
      label: "agent idle",
      timeoutMs: 5,
      pollMs: 1,
      observe: async () => ({ agent: "agent-1", state: "Working" }),
      isReady: () => false,
    }),
    /timed out waiting for agent idle\nlast_observation=/,
  )
})

test("waitForCondition reports transient observer errors", async () => {
  await assert.rejects(
    () => waitForCondition({
      label: "relay freshness",
      timeoutMs: 5,
      pollMs: 1,
      observe: async () => {
        throw new Error("relay unavailable")
      },
    }),
    /last_error=Error: relay unavailable/,
  )
})

test("waitForCondition can fail immediately on definitive errors", async () => {
  let count = 0
  await assert.rejects(
    () => waitForCondition({
      label: "file content",
      timeoutMs: 100,
      pollMs: 1,
      retryOnError: false,
      observe: async () => ({ actual: "wrong" }),
      isReady: () => {
        count += 1
        throw new Error("unexpected content")
      },
    }),
    /unexpected content/,
  )
  assert.equal(count, 1)
})

test("waitForTcpPort waits for reachable listeners", async () => {
  const server = await listenOnEphemeralPort()
  try {
    await waitForTcpPort(server.address().port, "127.0.0.1", 100)
  } finally {
    await closeServer(server)
  }
})

test("waitForTcpPort reports the last reachability observation", async () => {
  const server = await listenOnEphemeralPort()
  const port = server.address().port
  await closeServer(server)

  await assert.rejects(
    () => waitForTcpPort(port, "127.0.0.1", 5),
    /last_observation=/,
  )
})

test("findMatchingProcessIdsFromPsOutput finds run-owned drill processes", () => {
  const psOutput = `
  101 /usr/bin/login -fq user /opt/homebrew/bin/bun /repo/apps/cli/dist/index.js opencode --relay-token remote-native-token-4242 --automation-socket /tmp/arb-rnt-opencode-4242.sock
  102 /opt/homebrew/bin/bun /repo/apps/cli/dist/index.js --client-id arroba-remote-native-observer-opencode-4242 --automation-socket /tmp/arb-rnt-opencode-4242.sock
  103 /Applications/Codex.app/Contents/MacOS/Codex
  `

  assert.deepEqual(
    findMatchingProcessIdsFromPsOutput(psOutput, [
      "remote-native-token-4242",
      "/tmp/arb-rnt-opencode-4242.sock",
    ], 999),
    [101, 102],
  )
})

test("findMatchingProcessIdsFromPsOutput ignores the current process", () => {
  const psOutput = `
  201 node apps/cli/scripts/live-remote-native-tui-drill.mjs --relay-token remote-native-token-4242
  202 bun /repo/apps/cli/dist/index.js codex --relay-token remote-native-token-4242
  `

  assert.deepEqual(
    findMatchingProcessIdsFromPsOutput(psOutput, ["remote-native-token-4242"], 201),
    [202],
  )
})

test("findMatchingProcessIdsFromPsOutput supports regex markers and empty patterns", () => {
  const psOutput = `
  301 screen -dmS arroba-rnt-codex-a-4242 -L bun /repo/apps/cli/dist/index.js
  302 screen -dmS unrelated -L node server.js
  `

  assert.deepEqual(
    findMatchingProcessIdsFromPsOutput(psOutput, [
      "",
      null,
      /arroba-rnt-codex-[ab]-4242/,
    ], 999),
    [301],
  )
})

async function listenOnEphemeralPort() {
  const server = net.createServer()
  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, "127.0.0.1", resolve)
  })
  return server
}

async function closeServer(server) {
  await new Promise((resolve) => server.close(resolve))
}
