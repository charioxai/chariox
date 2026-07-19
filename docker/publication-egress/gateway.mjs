#!/usr/bin/env node

import { promises as dns } from "node:dns"
import { readFile } from "node:fs/promises"
import { createServer } from "node:http"
import { connect as connectTcp } from "node:net"
import process from "node:process"

import {
  canonicalDnsName,
  publicationEgressDestination,
  validatePublicationEgressPolicy,
  validateResolvedPublicationAddresses,
} from "./policy.mjs"
import { tlsClientHelloServerName } from "./tls-client-hello.mjs"

const DEFAULT_PORT = 3128
const MAX_CONCURRENT_TUNNELS = 256
const MAX_CLIENT_HELLO_BYTES = 65_536
const CLIENT_HELLO_TIMEOUT_MS = 5_000
const CONNECT_TIMEOUT_MS = 5_000
const TUNNEL_IDLE_TIMEOUT_MS = 300_000

export async function createPublicationEgressGateway({
  policy,
  host = "0.0.0.0",
  port = DEFAULT_PORT,
  resolveHost = resolvePublicHost,
  connectAddress = connectPublicAddress,
  log = defaultLog,
} = {}) {
  const validatedPolicy = validatePublicationEgressPolicy(policy)
  const activeTunnels = new Set()
  const server = createServer((request, response) => {
    if (request.method === "GET" && request.url === "/healthz") {
      response.writeHead(200, { "content-type": "application/json", "cache-control": "no-store" })
      response.end(JSON.stringify({ ok: true, policy_digest: validatedPolicy.policy_digest }))
      return
    }
    response.writeHead(405, { "content-type": "application/json", "cache-control": "no-store" })
    response.end('{"error":"ordinary HTTP proxy requests are disabled"}')
  })

  server.on("connect", (request, clientSocket, head) => {
    if (activeTunnels.size >= MAX_CONCURRENT_TUNNELS) {
      writeProxyFailure(clientSocket, 503, "capacity")
      return
    }
    activeTunnels.add(clientSocket)
    clientSocket.once("close", () => activeTunnels.delete(clientSocket))
    void handleConnect({
      policy: validatedPolicy,
      request,
      clientSocket,
      head,
      resolveHost,
      connectAddress,
      log,
    })
  })

  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(port, host, () => {
      server.removeListener("error", reject)
      resolve()
    })
  })

  return {
    server,
    policy: validatedPolicy,
    address: server.address(),
    async close() {
      for (const socket of activeTunnels) socket.destroy()
      await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()))
    },
  }
}

export async function authorizePublicationEgressTunnel({ policy, authority, clientHello, resolveHost = resolvePublicHost }) {
  const { host, port } = parseConnectAuthority(authority)
  const destination = publicationEgressDestination(policy, host, port)
  const serverName = canonicalDnsName(tlsClientHelloServerName(clientHello))
  if (serverName !== host) throw new Error("publication egress TLS SNI does not match CONNECT host")
  const addresses = validateResolvedPublicationAddresses(await resolveHost(host))
  return { destination, host, port, addresses }
}

async function handleConnect({ policy, request, clientSocket, head, resolveHost, connectAddress, log }) {
  let target
  let destination
  let addresses
  let responseStarted = false
  try {
    target = parseConnectAuthority(request.url)
    destination = publicationEgressDestination(policy, target.host, target.port)
    addresses = validateResolvedPublicationAddresses(await resolveHost(target.host))
    clientSocket.write("HTTP/1.1 200 Connection Established\r\nProxy-Agent: arroba-egress-v1\r\n\r\n")
    responseStarted = true
    const clientHello = await readTlsClientHello(clientSocket, head)
    const serverName = canonicalDnsName(tlsClientHelloServerName(clientHello.buffer))
    if (serverName !== target.host) throw new Error("publication egress TLS SNI does not match CONNECT host")
    const upstream = await connectFirstAddress(addresses, target.port, connectAddress)
    configureTunnelSocket(clientSocket)
    configureTunnelSocket(upstream)
    upstream.write(clientHello.buffer)
    clientSocket.pipe(upstream)
    upstream.pipe(clientSocket)
    const closeBoth = () => {
      clientSocket.destroy()
      upstream.destroy()
    }
    clientSocket.once("error", closeBoth)
    upstream.once("error", closeBoth)
    log({ event: "egress_allowed", destination_id: destination.id, host: target.host, port: target.port })
  } catch (error) {
    const reason = egressFailureReason(error)
    log({
      event: "egress_denied",
      destination_id: destination?.id ?? null,
      host: target?.host ?? null,
      port: target?.port ?? null,
      reason,
    })
    if (responseStarted) clientSocket.destroy()
    else writeProxyFailure(clientSocket, reason === "unavailable" ? 502 : 403, reason)
  }
}

export function parseConnectAuthority(value) {
  if (typeof value !== "string" || value.length > 512) throw new Error("publication egress CONNECT authority is invalid")
  const match = /^([a-z0-9.-]+):(\d{1,5})$/.exec(value)
  if (!match) throw new Error("publication egress CONNECT authority is invalid")
  const host = canonicalDnsName(match[1])
  const port = Number(match[2])
  if (port !== 443) throw new Error("publication egress permits TLS port 443 only")
  return { host, port }
}

async function resolvePublicHost(host) {
  const [ipv4, ipv6] = await Promise.allSettled([
    dns.resolve4(host),
    dns.resolve6(host),
  ])
  const addresses = []
  if (ipv4.status === "fulfilled") addresses.push(...ipv4.value.map((address) => ({ address, family: 4 })))
  if (ipv6.status === "fulfilled") addresses.push(...ipv6.value.map((address) => ({ address, family: 6 })))
  if (addresses.length === 0) throw new Error("publication egress destination is unavailable")
  return addresses
}

async function connectFirstAddress(addresses, port, connectAddress) {
  let lastError = null
  for (const address of addresses) {
    try {
      return await connectAddress(address, port)
    } catch (error) {
      lastError = error
    }
  }
  throw lastError ?? new Error("publication egress destination is unavailable")
}

function connectPublicAddress(address, port) {
  return new Promise((resolve, reject) => {
    const socket = connectTcp({ host: address.address, family: address.family, port })
    const timer = setTimeout(() => {
      socket.destroy()
      reject(new Error("publication egress upstream connect timed out"))
    }, CONNECT_TIMEOUT_MS)
    timer.unref?.()
    const onError = (error) => {
      clearTimeout(timer)
      reject(error)
    }
    socket.once("connect", () => {
      clearTimeout(timer)
      socket.removeListener("error", onError)
      resolve(socket)
    })
    socket.once("error", onError)
  })
}

function readTlsClientHello(socket, initial) {
  return new Promise((resolve, reject) => {
    let buffer = initial?.length ? Buffer.from(initial) : Buffer.alloc(0)
    let settled = false
    const timer = setTimeout(() => finish(new Error("publication egress TLS ClientHello timed out")), CLIENT_HELLO_TIMEOUT_MS)
    timer.unref?.()
    const finish = (error, value) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      socket.removeListener("data", onData)
      socket.removeListener("error", onError)
      socket.removeListener("close", onClose)
      if (error) reject(error)
      else {
        socket.pause()
        resolve(value)
      }
    }
    const inspect = () => {
      if (buffer.length > MAX_CLIENT_HELLO_BYTES) {
        finish(new Error("publication egress TLS ClientHello is too large"))
        return
      }
      try {
        const serverName = tlsClientHelloServerName(buffer)
        if (serverName !== null) finish(null, { buffer, serverName })
      } catch (error) {
        finish(error)
      }
    }
    const onData = (chunk) => {
      buffer = Buffer.concat([buffer, chunk], buffer.length + chunk.length)
      inspect()
    }
    const onError = () => finish(new Error("publication egress client tunnel failed"))
    const onClose = () => finish(new Error("publication egress client tunnel closed before TLS"))
    socket.on("data", onData)
    socket.once("error", onError)
    socket.once("close", onClose)
    inspect()
  })
}

function configureTunnelSocket(socket) {
  socket.setTimeout(TUNNEL_IDLE_TIMEOUT_MS, () => socket.destroy())
  socket.setNoDelay?.(true)
  socket.setKeepAlive?.(true, 30_000)
}

function writeProxyFailure(socket, status, reason) {
  if (socket.destroyed) return
  const label = status === 503 ? "Service Unavailable" : status === 502 ? "Bad Gateway" : "Forbidden"
  const body = JSON.stringify({ error: `egress_${reason}` })
  socket.end(
    `HTTP/1.1 ${status} ${label}\r\nContent-Type: application/json\r\nContent-Length: ${Buffer.byteLength(body)}\r\nConnection: close\r\n\r\n${body}`,
  )
}

function egressFailureReason(error) {
  const message = error instanceof Error ? error.message : String(error)
  if (/resolve|unavailable|timed out|connect/i.test(message)) return "unavailable"
  if (/SNI/i.test(message)) return "sni_mismatch"
  if (/TLS|ClientHello/i.test(message)) return "invalid_tls"
  if (/destination|authority|host|port|address|policy/i.test(message)) return "policy_denied"
  return "denied"
}

function defaultLog(entry) {
  process.stderr.write(`${JSON.stringify({ at: new Date().toISOString(), ...entry })}\n`)
}

async function main() {
  const policyPath = process.env.ARROBA_PUBLICATION_EGRESS_POLICY_FILE
  if (!policyPath) throw new Error("ARROBA_PUBLICATION_EGRESS_POLICY_FILE is required")
  const policy = JSON.parse(await readFile(policyPath, "utf8"))
  const gateway = await createPublicationEgressGateway({
    policy,
    host: process.env.HOST ?? "0.0.0.0",
    port: Number(process.env.PORT ?? DEFAULT_PORT),
  })
  defaultLog({ event: "egress_gateway_ready", policy_digest: gateway.policy.policy_digest })
  const stop = async (signal) => {
    defaultLog({ event: "egress_gateway_stopping", signal })
    await gateway.close()
  }
  process.once("SIGTERM", () => void stop("SIGTERM"))
  process.once("SIGINT", () => void stop("SIGINT"))
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    defaultLog({ event: "egress_gateway_failed", reason: egressFailureReason(error) })
    process.exitCode = 1
  })
}
