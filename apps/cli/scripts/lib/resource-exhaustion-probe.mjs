#!/usr/bin/env node

import { spawn } from "node:child_process"
import { closeSync, openSync } from "node:fs"
import { createServer, createConnection } from "node:net"

const mode = readMode(process.argv.slice(2))
const marker = readMarker(process.argv.slice(2))
const openedFiles = []
const children = []
let server
let client
let exhausted = false
let errorCode = null
let terminalLaneLive = false
let cleanupComplete = false

try {
  const lane = await openTerminalLane()
  server = lane.server
  client = lane.client

  if (mode === "file-descriptor") {
    while (openedFiles.length < 4096) {
      try {
        openedFiles.push(openSync("/dev/null", "r"))
      } catch (error) {
        errorCode = error?.code ?? "UNKNOWN"
        exhausted = new Set(["EMFILE", "ENFILE"]).has(errorCode)
        break
      }
    }
  } else {
    for (let index = 0; index < 32; index += 1) {
      const result = await spawnProbeChild(index, marker)
      if (result.error) {
        errorCode = result.error.code ?? "UNKNOWN"
        exhausted = errorCode === "EAGAIN"
        break
      }
      children.push(result.child)
    }
  }

  terminalLaneLive = await roundTrip(client)
} finally {
  for (const fd of openedFiles.splice(0)) closeSync(fd)
  for (const child of children) child.kill("SIGTERM")
  await Promise.all(children.map(waitForExit))
  client?.destroy()
  await closeServer(server)
  cleanupComplete = openedFiles.length === 0 && children.every((child) => child.exitCode !== null || child.signalCode !== null)
}

console.log(JSON.stringify({
  schema: "chariox.resource_exhaustion_probe.v1",
  mode,
  exhausted,
  errorCode: errorCode ?? "UNENFORCED",
  terminalLaneLive,
  cleanupComplete,
}))
if (!exhausted || !terminalLaneLive || !cleanupComplete) process.exitCode = 1

function readMode(argv) {
  const index = argv.indexOf("--mode")
  const value = index >= 0 ? argv[index + 1] : null
  if (!new Set(["file-descriptor", "process"]).has(value)) {
    throw new Error("--mode must be file-descriptor or process")
  }
  return value
}

function readMarker(argv) {
  const index = argv.indexOf("--marker")
  const value = index >= 0 ? argv[index + 1] : `chariox-resource-probe-${process.pid}`
  if (!/^chariox-resource-probe-[a-zA-Z0-9-]{1,120}$/.test(value)) {
    throw new Error("--marker is invalid")
  }
  return value
}

async function openTerminalLane() {
  let acceptConnection
  const accepted = new Promise((resolvePromise) => { acceptConnection = resolvePromise })
  const laneServer = createServer((socket) => {
    socket.on("data", (data) => {
      if (data.toString() === "terminal-ping") socket.write("terminal-pong")
    })
    acceptConnection()
  })
  laneServer.listen(0, "127.0.0.1")
  await onceEvent(laneServer, "listening")
  const address = laneServer.address()
  const laneClient = createConnection({ host: "127.0.0.1", port: address.port })
  await onceEvent(laneClient, "connect")
  await accepted
  if (!await roundTrip(laneClient)) throw new Error("terminal lane failed before resource exhaustion")
  return { server: laneServer, client: laneClient }
}

function roundTrip(socket) {
  return new Promise((resolvePromise) => {
    const timer = setTimeout(() => resolvePromise(false), 2_000)
    socket.once("data", (data) => {
      clearTimeout(timer)
      resolvePromise(data.toString() === "terminal-pong")
    })
    socket.write("terminal-ping")
  })
}

function spawnProbeChild(index, processMarker) {
  return new Promise((resolvePromise) => {
    const child = spawn(process.execPath, ["-e", "setTimeout(() => process.exit(0), 5000)", `${processMarker}-${index}`], {
      stdio: "ignore",
    })
    child.once("spawn", () => resolvePromise({ child }))
    child.once("error", (error) => resolvePromise({ child, error }))
  })
}

function waitForExit(child) {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve()
  return onceEvent(child, "exit").catch(() => undefined)
}

function closeServer(value) {
  if (!value) return Promise.resolve()
  return new Promise((resolvePromise) => value.close(resolvePromise))
}

function onceEvent(emitter, event) {
  return new Promise((resolvePromise, rejectPromise) => {
    emitter.once(event, (...args) => resolvePromise(args))
    emitter.once("error", rejectPromise)
  })
}
