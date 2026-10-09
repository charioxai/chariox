import assert from "node:assert/strict"
import test from "node:test"
import { runAppCommand } from "./app-command.js"

function harness(response: Record<string, unknown> = {
  AppInstallationsListed: { installations: [], next_cursor: null },
}) {
  const connections: unknown[] = []
  const requests: unknown[] = []
  const output: string[] = []
  let closed = 0
  return {
    connections, requests, output,
    get closed() { return closed },
    deps: {
      createClient: (endpoint: string, options: unknown) => {
        connections.push({ endpoint, options })
        return {
          send: async (request: Record<string, unknown>) => { requests.push(request); return response },
          close: async () => { closed += 1 },
        }
      },
      write: (message: string) => { output.push(message) },
    },
  }
}

test("standalone App list uses ordinary direct kernel transport without a session", async () => {
  const h = harness()
  assert.equal(await runAppCommand(["app", "list", "--limit", "10", "--after", "install-1", "--kernel-url", "ws://localhost:43118/kernel"], h.deps), true)
  assert.deepEqual(h.connections, [{ endpoint: "ws://localhost:43118/kernel", options: {} }])
  assert.deepEqual(h.requests, [{ ListAppInstallations: { after: "install-1", limit: 10 } }])
  assert.deepEqual(h.output, ["No App installations.\n"])
  assert.equal(h.closed, 1)
})

test("standalone App journal preserves relay target and authorization options", async () => {
  const h = harness({ AppInstallationJournal: { installation_id: "install-1", updates: [] } })
  await runAppCommand(["app", "journal", "install-1", "--relay-url", "wss://relay.example.test", "--relay-token", "fixture-token", "--target-daemon-id", "kernel-1"], h.deps)
  assert.deepEqual(h.connections, [{ endpoint: "wss://relay.example.test", options: { relayAuthToken: "fixture-token", targetDaemonId: "kernel-1" } }])
  assert.deepEqual(h.requests, [{ GetAppInstallationJournal: { installation_id: "install-1" } }])
  assert.equal(h.closed, 1)
})

test("standalone App command rejects unsupported actions and invalid flags before connecting", async () => {
  for (const args of [
    ["install", "path.cxapp"], ["update", "install-1", "path.cxapp"], ["open", "install-1"],
    ["list", "--limit", "101"], ["status"], ["journal", "one", "two"],
    ["list", "--session", "session-1"], ["list", "--kernel-url"],
    ["list", "--relay-url", "wss://relay.example.test"],
    ["list", "--kernel-url", "ws://one", "--kernel-url", "ws://two"],
    ["list", "--socket", "/tmp/kernel", "--kernel-url", "ws://one"],
    ["list", "--relay-token", "fixture-token"], ["list", "--target-daemon-id", "kernel-1"],
    ["list", "--pairing-link", "one", "--terminal-pairing-link", "two"],
    ["list", "--pairing-link", "one", "--target-daemon-id", "kernel-2"],
  ]) {
    const h = harness()
    await assert.rejects(runAppCommand(["app", ...args], h.deps))
    assert.deepEqual(h.connections, [], args.join(" "))
  }
})

test("standalone App install and update name a session and read the file before connecting", async () => {
  for (const args of [["install", "missing.cxapp"], ["install", "missing.cxapp", "--session"],
    ["update", "missing.cxapp", "--session", "s1"], ["install", "a.cxapp", "b.cxapp", "--session", "s1"]]) {
    const h = harness()
    await assert.rejects(runAppCommand(["app", ...args], h.deps), /usage: app install FILE\.cxapp --session SESSION/)
    assert.deepEqual(h.connections, [], args.join(" "))
  }
  const h = harness()
  await assert.rejects(runAppCommand(["app", "install", "/nonexistent/app.cxapp", "--session", "s1"], h.deps))
  assert.deepEqual(h.connections, [])
})

test("standalone App command accepts an existing terminal pairing link", async () => {
  const h = harness()
  const link = "chariox-terminal-pair-v1." + Buffer.from(JSON.stringify({
    relay_url: "wss://relay.example.test", relay_token: "fixture-token",
    target_daemon_id: "kernel-1", terminal_id: "terminal-1",
  })).toString("base64url")
  await runAppCommand(["app", "list", "--terminal-pairing-link", link], h.deps)
  assert.deepEqual(h.connections, [{ endpoint: "wss://relay.example.test", options: { relayAuthToken: "fixture-token", targetDaemonId: "kernel-1" } }])
  assert.equal(h.closed, 1)
})

test("standalone App command closes after kernel denials and transport failures", async () => {
  const denied = harness({ AppRequestFailed: { code: "unauthorized" } })
  await assert.rejects(runAppCommand(["app", "list"], denied.deps), /not authorized/)
  assert.equal(denied.closed, 1)
  assert.deepEqual(denied.output, [])

  let closed = false
  await assert.rejects(runAppCommand(["app", "status", "install-1"], {
    createClient: () => ({
      send: async () => { throw new Error("connection lost") },
      close: async () => { closed = true },
    }),
    write: () => assert.fail("failed request must not emit success"),
  }), /connection lost/)
  assert.equal(closed, true)
})

test("other standalone commands are not intercepted", async () => {
  const h = harness()
  assert.equal(await runAppCommand(["apps", "list"], h.deps), false)
  assert.deepEqual(h.connections, [])
})


test("standalone App quarantine names the explicit start recovery command", async () => {
  const h = harness({ AppWorker: { worker: { installation_id: "install-1", phase: "quarantined",
    enabled: true, failure: "app_worker_exited", updated_at_ms: 1 } } })
  await runAppCommand(["app", "worker", "install-1"], h.deps)
  assert.deepEqual(h.requests, [{ GetAppWorker: { installation_id: "install-1" } }])
  assert.deepEqual(h.output, ['install-1 · quarantined · explicit start required: chariox app start "install-1" · app_worker_exited\n'])
  assert.equal(h.closed, 1)
  const recovered = harness({ AppWorker: { worker: { installation_id: "install-1", phase: "running",
    enabled: true, failure: null, updated_at_ms: 2 } } })
  await runAppCommand(["app", "start", "install-1"], recovered.deps)
  assert.deepEqual(recovered.requests, [{ ControlAppWorker: { installation_id: "install-1", action: "start" } }])
  assert.deepEqual(recovered.output, ["install-1 · running\n"])
})

for (const transport of [[], ["--relay-url", "wss://relay.example.test", "--relay-token", "fixture-token", "--target-daemon-id", "kernel-1"]]) {
  test(`standalone CLI refuses expired App replay over ${transport.length ? "relay" : "local"} transport`, async () => {
    const h = harness({ AppRequestFailed: { code: "receipt_expired" } })
    await assert.rejects(runAppCommand(["app", "restart", "install-1", ...transport], h.deps),
      /App receipt expired\. This request was not re-executed; check the App state before issuing a new command\./)
    assert.deepEqual(h.requests, [{ ControlAppWorker: { installation_id: "install-1", action: "restart" } }])
    assert.equal(h.closed, 1)
    assert.deepEqual(h.output, [])
  })
}

test("standalone App list extracts both original issuer values before dispatch", async () => {
  const h = harness()
  await runAppCommand(["app", "list", "--relay-url", "wss://relay.example.test", "--relay-token", "fixture-token", "--target-daemon-id", "managed-kernel", "--relay-token-issuer", "ws://127.0.0.1:49911/kernel", "account-kernel", "--limit", "10"], h.deps)
  assert.deepEqual(h.connections, [{ endpoint: "wss://relay.example.test", options: {
    relayAuthToken: "fixture-token", targetDaemonId: "managed-kernel",
    relayAuthorizationIssuer: { endpoint: "ws://127.0.0.1:49911/kernel", daemonId: "account-kernel" },
  } }])
  assert.deepEqual(h.requests, [{ ListAppInstallations: { after: null, limit: 10 } }])
  assert.equal(h.closed, 1)
})

for (const [name, issuerArgs, message] of [
  ["missing endpoint", ["--relay-token-issuer"], /missing value for --relay-token-issuer/],
  ["missing kernel ID", ["--relay-token-issuer", "ws://127.0.0.1/kernel"], /missing value for --relay-token-issuer/],
  ["next flag instead of kernel ID", ["--relay-token-issuer", "ws://127.0.0.1/kernel", "--limit", "10"], /missing value for --relay-token-issuer/],
  ["duplicate issuer", ["--relay-token-issuer", "ws://127.0.0.1/kernel", "one", "--relay-token-issuer", "ws://127.0.0.1/kernel", "two"], /duplicate connection option --relay-token-issuer/],
] as const) test(`standalone App rejects ${name} before connecting`, async () => {
  const h = harness()
  await assert.rejects(runAppCommand(["app", "list", "--relay-url", "wss://relay.example.test", "--relay-token", "fixture-token", "--target-daemon-id", "managed-kernel", ...issuerArgs], h.deps), message)
  assert.deepEqual(h.connections, [])
})
