import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { once } from "node:events"
import { connect, createServer as createNetServer } from "node:net"
import test from "node:test"

import {
  authorizePublicationEgressTunnel,
  createPublicationEgressGateway,
  parseConnectAuthority,
} from "./gateway.mjs"
import { buildPublicationEgressPolicy } from "./policy.mjs"
import { tlsClientHello } from "./test-helpers.mjs"

const policy = buildPublicationEgressPolicy([
  { id: "provider:openai", host: "api.openai.com", ports: [443] },
])

test("publication egress survives a client reset while authorizing a tunnel", { timeout: 10_000 }, async () => {
  const child = spawn(process.execPath, ["--input-type=module", "--eval", `
    import { createPublicationEgressGateway } from ${JSON.stringify(new URL("./gateway.mjs", import.meta.url).href)};
    const gateway = await createPublicationEgressGateway({
      policy: ${JSON.stringify(policy)}, host: "127.0.0.1", port: 0,
      resolveHost: async () => {
        process.send({ resolving: true });
        await new Promise(resolve => process.once("message", resolve));
        throw new Error("publication egress destination is unavailable");
      },
      log: () => {},
    });
    process.send({ port: gateway.address.port });
  `], { stdio: ["ignore", "ignore", "pipe", "ipc"] })
  let stderr = ""
  child.stderr.setEncoding("utf8").on("data", chunk => { stderr += chunk })
  const exited = once(child, "exit")
  const failure = exited.then(([code, signal]) => { throw new Error(`gateway exited ${code}/${signal}: ${stderr}`) })
  // Observe rejection immediately, including while the client socket is being reset.
  failure.catch(() => {})
  let socket
  try {
    const [{ port }] = await Promise.race([once(child, "message"), failure])
    const resolving = once(child, "message")
    socket = connect(port, "127.0.0.1")
    socket.on("error", () => {})
    await once(socket, "connect")
    socket.write("CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com:443\r\n\r\n")
    assert.deepEqual((await Promise.race([resolving, failure]))[0], { resolving: true })
    const closed = once(socket, "close")
    socket.resetAndDestroy()
    await closed
    // Let the kernel deliver the TCP reset while DNS authorization is still pending.
    await new Promise(resolve => setTimeout(resolve, 50))
    await Promise.race([
      (async () => {
        const health = await fetch(`http://127.0.0.1:${port}/healthz`)
        assert.equal(health.status, 200)
        assert.equal((await health.json()).policy_digest, policy.policy_digest)
      })(),
      failure,
    ])
    child.send({ finishAuthorization: true })
  } finally {
    socket?.destroy()
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGTERM")
    await exited
  }
})

test("publication egress authorization binds CONNECT, SNI, policy, and all DNS answers", async () => {
  const authorized = await authorizePublicationEgressTunnel({
    policy,
    authority: "api.openai.com:443",
    clientHello: tlsClientHello("api.openai.com"),
    resolveHost: async () => [
      { address: "8.8.8.8", family: 4 },
      { address: "2001:4860:4860::8888", family: 6 },
    ],
  })
  assert.equal(authorized.destination.id, "provider:openai")
  assert.equal(authorized.addresses.length, 2)

  await assert.rejects(authorizePublicationEgressTunnel({
    policy,
    authority: "api.openai.com:443",
    clientHello: tlsClientHello("api.anthropic.com"),
    resolveHost: async () => [{ address: "8.8.8.8", family: 4 }],
  }), /SNI does not match/)
  await assert.rejects(authorizePublicationEgressTunnel({
    policy,
    authority: "api.openai.com:443",
    clientHello: tlsClientHello("api.openai.com"),
    resolveHost: async () => [{ address: "169.254.169.254", family: 4 }],
  }), /forbidden address/)
})

test("publication egress CONNECT parser rejects alternate ports and ambiguous authorities", () => {
  assert.deepEqual(parseConnectAuthority("api.openai.com:443"), { host: "api.openai.com", port: 443 })
  for (const authority of ["api.openai.com:80", "user@api.openai.com:443", "127.0.0.1:443", "api.openai.com"]){
    assert.throws(() => parseConnectAuthority(authority))
  }
})

test("publication egress gateway exposes health but denies ordinary proxying and unknown CONNECT hosts", async () => {
  let resolverCalls = 0
  const logs = []
  const gateway = await createPublicationEgressGateway({
    policy,
    host: "127.0.0.1",
    port: 0,
    resolveHost: async () => {
      resolverCalls += 1
      return [{ address: "8.8.8.8", family: 4 }]
    },
    log: (entry) => logs.push(entry),
  })
  const address = gateway.address
  assert.ok(address && typeof address === "object")
  try {
    const health = await fetch(`http://127.0.0.1:${address.port}/healthz`)
    assert.equal(health.status, 200)
    assert.equal((await health.json()).policy_digest, policy.policy_digest)

    const ordinary = await fetch(`http://127.0.0.1:${address.port}/https://api.openai.com/`)
    assert.equal(ordinary.status, 405)

    const response = await rawRequest(address.port, "CONNECT metadata.google.internal:443 HTTP/1.1\r\nHost: metadata.google.internal:443\r\n\r\n")
    assert.match(response, /^HTTP\/1\.1 403 Forbidden/)
    assert.equal(resolverCalls, 0)
    assert.equal(logs.at(-1)?.event, "egress_denied")
  } finally {
    await gateway.close()
  }
})

test("publication egress gateway pins an approved address and tunnels only after matching SNI", async () => {
  const hello = tlsClientHello("api.openai.com")
  let upstreamBytes = null
  const upstream = createNetServer((socket) => {
    socket.once("data", (chunk) => {
      upstreamBytes = Buffer.from(chunk)
      socket.end("upstream-ok")
    })
  })
  await new Promise((resolve, reject) => {
    upstream.once("error", reject)
    upstream.listen(0, "127.0.0.1", resolve)
  })
  const upstreamAddress = upstream.address()
  assert.ok(upstreamAddress && typeof upstreamAddress === "object")
  const connectedAddresses = []
  const gateway = await createPublicationEgressGateway({
    policy,
    host: "127.0.0.1",
    port: 0,
    resolveHost: async () => [{ address: "8.8.8.8", family: 4 }],
    connectAddress: async (address) => {
      connectedAddresses.push(address)
      return await new Promise((resolve, reject) => {
        const socket = connect(upstreamAddress.port, "127.0.0.1")
        socket.once("connect", () => resolve(socket))
        socket.once("error", reject)
      })
    },
    log: () => {},
  })
  const gatewayAddress = gateway.address
  assert.ok(gatewayAddress && typeof gatewayAddress === "object")
  try {
    const response = await connectThroughGateway(gatewayAddress.port, hello)
    assert.match(response, /^HTTP\/1\.1 200 Connection Established/)
    assert.match(response, /upstream-ok$/)
    assert.deepEqual(connectedAddresses, [{ address: "8.8.8.8", family: 4 }])
    assert.deepEqual(upstreamBytes, hello)
  } finally {
    await gateway.close()
    await new Promise((resolve, reject) => upstream.close((error) => error ? reject(error) : resolve()))
  }
})

function rawRequest(port, request) {
  return new Promise((resolve, reject) => {
    const socket = connect(port, "127.0.0.1")
    const chunks = []
    socket.once("connect", () => socket.end(request))
    socket.on("data", (chunk) => chunks.push(chunk))
    socket.once("error", reject)
    socket.once("close", () => resolve(Buffer.concat(chunks).toString("utf8")))
  })
}

function connectThroughGateway(port, hello) {
  return new Promise((resolve, reject) => {
    const socket = connect(port, "127.0.0.1")
    const chunks = []
    let sentHello = false
    socket.once("connect", () => socket.write("CONNECT api.openai.com:443 HTTP/1.1\r\nHost: api.openai.com:443\r\n\r\n"))
    socket.on("data", (chunk) => {
      chunks.push(chunk)
      const response = Buffer.concat(chunks).toString("utf8")
      if (!sentHello && response.includes("\r\n\r\n")) {
        sentHello = true
        socket.write(hello)
      }
    })
    socket.once("error", reject)
    socket.once("close", () => resolve(Buffer.concat(chunks).toString("utf8")))
  })
}
