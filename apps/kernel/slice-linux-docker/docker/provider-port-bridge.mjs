#!/usr/bin/env node

import { readFile, writeFile } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import process from "node:process"
import { pathToFileURL } from "node:url"

export function parsePortRanges(value) {
  const ports = new Set()
  for (const rawRange of String(value ?? "").split(",")) {
    const range = rawRange.trim()
    if (!range) continue
    const match = /^(\d+)(?:-(\d+))?$/.exec(range)
    if (!match) throw new Error(`invalid provider bridge port range: ${range}`)
    const start = Number(match[1])
    const end = Number(match[2] ?? match[1])
    if (!Number.isInteger(start) || !Number.isInteger(end) || start < 1 || end > 65_535 || start > end) {
      throw new Error(`invalid provider bridge port range: ${range}`)
    }
    for (let port = start; port <= end; port += 1) ports.add(port)
    if (ports.size > 256) throw new Error("provider bridge cannot expose more than 256 ports")
  }
  if (ports.size === 0) throw new Error("provider bridge requires at least one port")
  return [...ports].sort((left, right) => left - right)
}

export function defaultRouteFromProc(text) {
  for (const line of String(text).trim().split("\n").slice(1)) {
    const fields = line.trim().split(/\s+/)
    if (fields.length < 4 || fields[1] !== "00000000") continue
    const flags = Number.parseInt(fields[3], 16)
    if (!Number.isFinite(flags) || (flags & 0x2) === 0) continue
    const gateway = hexLittleEndianIpv4(fields[2])
    if (gateway) return { interfaceName: fields[0], gateway }
  }
  throw new Error("provider bridge could not resolve the container default route")
}

export function interfaceIpv4(interfaceName, interfaces = os.networkInterfaces()) {
  const address = interfaces[interfaceName]?.find((candidate) =>
    candidate.family === "IPv4" && !candidate.internal)
  if (!address?.address) throw new Error(`provider bridge could not resolve IPv4 for ${interfaceName}`)
  return address.address
}

export function gatewaySourceMatches(remoteAddress, gateway) {
  return normalizeIpv4(remoteAddress) === normalizeIpv4(gateway)
}

export async function startProviderPortBridge({
  ports,
  bindHost,
  allowedSource,
  upstreamHost = "127.0.0.1",
}) {
  const servers = []
  const sockets = new Set()
  try {
    for (const port of ports) {
      const server = net.createServer((client) => {
        if (!gatewaySourceMatches(client.remoteAddress, allowedSource)) {
          client.destroy()
          return
        }
        sockets.add(client)
        const upstream = net.createConnection({ host: upstreamHost, port })
        sockets.add(upstream)
        const closePair = () => {
          client.destroy()
          upstream.destroy()
        }
        client.once("error", closePair)
        upstream.once("error", closePair)
        client.once("close", () => sockets.delete(client))
        upstream.once("close", () => sockets.delete(upstream))
        client.pipe(upstream)
        upstream.pipe(client)
      })
      await listen(server, bindHost, port)
      servers.push(server)
    }
  } catch (error) {
    await closeBridgeServers(servers, sockets)
    throw error
  }
  return {
    close: async () => closeBridgeServers(servers, sockets),
  }
}

async function main() {
  const ports = parsePortRanges(process.env.CHARIOX_SLICE_PROVIDER_BRIDGE_PORT_RANGES)
  const route = defaultRouteFromProc(await readFile("/proc/net/route", "utf8"))
  const bindHost = process.env.CHARIOX_SLICE_PROVIDER_BRIDGE_BIND_HOST?.trim()
    || interfaceIpv4(route.interfaceName)
  const allowedSource = process.env.CHARIOX_SLICE_PROVIDER_BRIDGE_ALLOWED_SOURCE?.trim()
    || route.gateway
  const bridge = await startProviderPortBridge({ ports, bindHost, allowedSource })
  const readyFile = process.env.CHARIOX_SLICE_PROVIDER_BRIDGE_READY_FILE?.trim()
  if (readyFile) {
    await writeFile(readyFile, `${JSON.stringify({ bindHost, allowedSource, ports })}\n`, { mode: 0o600 })
  }
  process.stderr.write(`[slice-provider-bridge] ready bind=${bindHost} source=${allowedSource} ports=${ports.length}\n`)
  await new Promise((resolve) => {
    process.once("SIGINT", resolve)
    process.once("SIGTERM", resolve)
  })
  await bridge.close()
}

function hexLittleEndianIpv4(value) {
  if (!/^[0-9A-Fa-f]{8}$/.test(value)) return null
  return value.match(/../g).reverse().map((byte) => Number.parseInt(byte, 16)).join(".")
}

function normalizeIpv4(value) {
  return String(value ?? "").replace(/^::ffff:/, "")
}

async function listen(server, host, port) {
  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen({ host, port, exclusive: true }, () => {
      server.off("error", reject)
      resolve()
    })
  })
}

async function closeBridgeServers(servers, sockets) {
  for (const socket of sockets) socket.destroy()
  await Promise.all(servers.map((server) => new Promise((resolve) => server.close(resolve))))
}

const entryUrl = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : null
if (entryUrl === import.meta.url) {
  main().catch((error) => {
    console.error(`[slice-provider-bridge] ${error.stack ?? error.message}`)
    process.exit(1)
  })
}
