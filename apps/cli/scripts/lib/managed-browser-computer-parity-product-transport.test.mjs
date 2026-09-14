import assert from "node:assert/strict"
import http from "node:http"
import { mkdtemp, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

const productTransportModule = new URL("./managed-browser-computer-parity-product-transport.mjs", import.meta.url)

test("managed parity product factory fails closed without an operator kernel endpoint", async () => {
  const evidenceRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-managed-parity-contract-"))
  const previousEndpoint = process.env.CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL
  try {
    const imported = await import(productTransportModule.href)
    assert.equal(typeof imported.createManagedBrowserComputerParityTransport, "function")

    delete process.env.CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL
    await assert.rejects(
      () => imported.createManagedBrowserComputerParityTransport({ evidenceRoot }),
      /CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL is required/,
    )
  } finally {
    if (previousEndpoint === undefined) delete process.env.CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL
    else process.env.CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL = previousEndpoint
    await rm(evidenceRoot, { recursive: true, force: true })
  }
})

test("managed parity product factory requires the standard local-auth configuration", async () => {
  const evidenceRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-managed-parity-auth-"))
  const names = [
    "CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL",
    "CHARIOX_MANAGED_PARITY_TARGET_KERNEL_REF",
    "CHARIOX_MANAGED_PARITY_TARGET_MACHINE_REF",
    "CHARIOX_MANAGED_PARITY_CLIENT_ID",
    "CHARIOX_MANAGED_PARITY_SESSION_ID",
    "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN",
    "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE",
  ]
  const previous = new Map(names.map((name) => [name, process.env[name]]))
  try {
    const imported = await import(productTransportModule.href)
    process.env.CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL = "ws://127.0.0.1:43118/"
    process.env.CHARIOX_MANAGED_PARITY_TARGET_KERNEL_REF = "kernel-ref"
    process.env.CHARIOX_MANAGED_PARITY_TARGET_MACHINE_REF = "machine-ref"
    process.env.CHARIOX_MANAGED_PARITY_CLIENT_ID = "client-id"
    process.env.CHARIOX_MANAGED_PARITY_SESSION_ID = "session-id"
    delete process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN
    delete process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE
    await assert.rejects(
      () => imported.createManagedBrowserComputerParityTransport({ evidenceRoot }),
      /requires CHARIOX_KERNEL_LOCAL_AUTH_TOKEN or CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE/,
    )
  } finally {
    for (const [name, value] of previous) {
      if (value === undefined) delete process.env[name]
      else process.env[name] = value
    }
    await rm(evidenceRoot, { recursive: true, force: true })
  }
})

test("managed parity selkies attach uses authenticated public responses and rejects unsupported steps", async (t) => {
  const imported = await import(productTransportModule.href)
  assert.equal(typeof imported.createManagedBrowserComputerParityTransportFromPublicClient, "function")

  const seenRequests = []
  const server = http.createServer(async (request, response) => {
    assert.equal(request.headers.authorization, "Bearer operator-test-token")
    assert.equal(request.headers["content-type"], "application/json")
    let body = ""
    for await (const chunk of request) body += chunk
    const payload = JSON.parse(body)
    seenRequests.push(payload)

    let result
    if (JSON.stringify(payload) === JSON.stringify({ RelayStatus: null })) {
      result = {
        RelayStatus: {
          status: {
            configured: true,
            connected: true,
            relay_token_configured: true,
            daemon_id: "kernel-1",
            machine_id: "machine-1",
          },
        },
      }
    } else if (JSON.stringify(payload) === JSON.stringify({
      GetRoomEnvironmentState: { session_id: "room-1" },
    })) {
      result = {
        RoomEnvironmentState: {
          environment: {
            session_id: "room-1",
            environment_id: "environment-1",
          },
        },
      }
    } else {
      response.writeHead(400, { "content-type": "application/json" })
      response.end(JSON.stringify({ error: "unexpected kernel request" }))
      return
    }
    response.writeHead(200, { "content-type": "application/json" })
    response.end(JSON.stringify(result))
  })
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve))
  t.after(() => new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve())))

  const address = server.address()
  assert(address && typeof address === "object")
  const publicClient = {
    async send(request, { signal } = {}) {
      const response = await fetch(`http://127.0.0.1:${address.port}/kernel`, {
        method: "POST",
        headers: {
          authorization: "Bearer operator-test-token",
          "content-type": "application/json",
        },
        body: JSON.stringify(request),
        signal,
      })
      const body = await response.json()
      if (!response.ok) throw new Error(body.error ?? `kernel request failed: ${response.status}`)
      return body
    },
  }
  const requestApi = {
    relayStatusRequest: () => ({ RelayStatus: null }),
    getRoomEnvironmentStateRequest: (sessionId) => ({
      GetRoomEnvironmentState: { session_id: sessionId },
    }),
  }
  const transport = imported.createManagedBrowserComputerParityTransportFromPublicClient({
    client: publicClient,
    requestApi,
  })
  const binding = {
    kernelId: "kernel-1",
    machineId: "machine-1",
    roomId: "room-1",
    environmentId: "environment-1",
  }

  const result = await transport.run("selkies.attach", {
    runId: "run-1",
    binding,
    client: "web",
    displayBackend: "selkies",
  }, { signal: new AbortController().signal })

  assert.deepEqual(seenRequests, [
    { RelayStatus: null },
    { GetRoomEnvironmentState: { session_id: "room-1" } },
  ])
  assert.deepEqual(result, {
    kernelId: "kernel-1",
    machineId: "machine-1",
    roomId: "room-1",
    environmentId: "environment-1",
    client: "web",
    displayBackend: "selkies",
  })

  await assert.rejects(
    () => transport.run("preflight", { runId: "run-1", binding }),
    /unsupported managed parity step: preflight/,
  )
  assert.equal(seenRequests.length, 2)
})

test("managed parity selkies attach propagates public-client errors and rejects response identity drift", async () => {
  const imported = await import(productTransportModule.href)
  const requestApi = {
    relayStatusRequest: () => ({ RelayStatus: null }),
    getRoomEnvironmentStateRequest: (sessionId) => ({
      GetRoomEnvironmentState: { session_id: sessionId },
    }),
  }
  const expectedError = new Error("operator endpoint unavailable")
  const errorTransport = imported.createManagedBrowserComputerParityTransportFromPublicClient({
    client: { send: async () => { throw expectedError } },
    requestApi,
  })
  await assert.rejects(
    () => errorTransport.run("selkies.attach", {
      binding: {
        kernelId: "kernel-1",
        machineId: "machine-1",
        roomId: "room-1",
        environmentId: "environment-1",
      },
      client: "web",
      displayBackend: "selkies",
    }),
    (error) => error === expectedError,
  )

  const driftTransport = imported.createManagedBrowserComputerParityTransportFromPublicClient({
    client: {
      async send(request) {
        if ("RelayStatus" in request) {
          return {
            RelayStatus: {
              status: {
                configured: true,
                connected: true,
                daemon_id: "other-kernel",
                machine_id: "machine-1",
              },
            },
          }
        }
        return {
          RoomEnvironmentState: {
            environment: {
              session_id: "room-1",
              environment_id: "environment-1",
            },
          },
        }
      },
    },
    requestApi,
  })
  await assert.rejects(
    () => driftTransport.run("selkies.attach", {
      binding: {
        kernelId: "kernel-1",
        machineId: "machine-1",
        roomId: "room-1",
        environmentId: "environment-1",
      },
      client: "web",
      displayBackend: "selkies",
    }),
    /managed parity target identity mismatch for selkies\.attach: kernelId/,
  )
})
