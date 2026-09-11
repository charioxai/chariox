#!/usr/bin/env node

import { connect } from "node:net"
import { basename } from "node:path"

function fail(message) {
  throw new Error(message)
}

function connectOnce(host, port, timeoutMs) {
  return new Promise((resolvePromise) => {
    const socket = connect({ host, port })
    const finish = (connected) => {
      socket.destroy()
      resolvePromise(connected)
    }
    socket.setTimeout(Math.min(timeoutMs, 500), () => finish(false))
    socket.once("connect", () => finish(true))
    socket.once("error", () => finish(false))
  })
}

async function check(host, portText, timeoutText) {
  if (!new Set(["127.0.0.1", "localhost", "::1"]).has(host)) {
    fail("managed kernel health host must be loopback")
  }
  const port = Number(portText)
  const timeoutMs = Number(timeoutText)
  if (!Number.isInteger(port) || port < 1 || port > 65535) fail("managed kernel health port is invalid")
  if (!Number.isInteger(timeoutMs) || timeoutMs < 100 || timeoutMs > 120_000) {
    fail("managed kernel health timeout is invalid")
  }
  const deadline = Date.now() + timeoutMs
  do {
    if (await connectOnce(host, port, deadline - Date.now())) return
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 100))
  } while (Date.now() < deadline)
  fail("managed kernel loopback listener did not become healthy")
}

try {
  if (process.argv.length !== 5) fail("usage: check-managed-kernel-health <host> <port> <timeout-ms>")
  await check(process.argv[2], process.argv[3], process.argv[4])
} catch (error) {
  const message = error instanceof Error ? error.message : String(error)
  process.stderr.write(`${basename(process.argv[1])}: ${message}\n`)
  process.exitCode = 1
}
