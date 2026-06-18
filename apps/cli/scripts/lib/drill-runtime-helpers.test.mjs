import assert from "node:assert/strict"
import net from "node:net"
import test from "node:test"

import { formatDrillCommandLine, runLogged, waitForCondition, waitForTcpPort } from "./drill-runtime-helpers.mjs"

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
