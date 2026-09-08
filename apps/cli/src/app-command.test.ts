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
    ["install", "path.cxapp"], ["open", "install-1"], ["restart", "install-1"],
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
