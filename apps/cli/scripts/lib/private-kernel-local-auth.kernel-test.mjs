import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { once } from "node:events"
import { mkdir, mkdtemp, readFile, readdir, rm } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import WebSocket from "ws"
import { LocalIpcClient } from "../../../../packages/kernel-client/dist/ipc.js"
import { attachToSessionRequest, createSessionRequest, listSessionsRequest } from "../../../../packages/kernel-client/dist/ipc-requests.js"
import { waitForKernelIpc } from "./live-provider-context-injection-drill-helpers.mjs"

for (const isolation of ["CHARIOX_HOME", "XDG_STATE_HOME"]) {
  test(`private ${isolation} clients authenticate under enforcement, including after restart`, {
    skip: !process.env.CHARIOX_LOCAL_AUTH_KERNEL_BINARY,
    timeout: 60_000,
  }, async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), "chariox-private-local-auth-"))
    const children = new Set()
    const clients = []
    try {
      const kernels = []
      for (const name of ["home", "worker"]) {
        const directory = path.join(root, name)
        await mkdir(directory, { recursive: true })
        const ports = await reservePorts()
        // Fresh profiles: this drill does not provision or launch provider accounts.
        const env = {
          PATH: process.env.PATH,
          HOME: path.join(directory, "home"),
          XDG_CONFIG_HOME: path.join(directory, "config"),
          XDG_STATE_HOME: path.join(directory, "state"),
          XDG_DATA_HOME: path.join(directory, "data"),
          XDG_CACHE_HOME: path.join(directory, "cache"),
          [isolation]: path.join(directory, "private"),
          CHARIOX_LOG_DIR: path.join(directory, "logs"),
          CHARIOX_LOG_LEVEL: "info",
          TOKIO_WORKER_THREADS: "1",
          CHARIOX_KERNEL_PORT: String(ports[0]),
          CHARIOX_MCP_PORT: String(ports[1]),
          CHARIOX_CODEX_PORT: String(ports[2]),
          CHARIOX_OPENCODE_PORT: String(ports[3]),
          CHARIOX_DAEMON_SOCKET: path.join(directory, "daemon.sock"),
          CHARIOX_DAEMON_ID: `local-auth-${name}`,
          CHARIOX_MACHINE_ID: `local-auth-${name}`,
        }
        const endpoint = `ws://127.0.0.1:${ports[0]}/kernel`
        const kernel = { env, endpoint, directory, output: "" }
        kernel.child = startKernel(kernel, children)
        await waitForKernelIpc(LocalIpcClient, listSessionsRequest, endpoint, kernel.child, 25_000, env)
        kernels.push(kernel)
      }
      for (const kernel of kernels) {
        const client = new LocalIpcClient(kernel.endpoint, { localAuthEnvironment: kernel.env })
        clients.push(client)
        await client.send(listSessionsRequest())
        const workspace = path.join(root, "workspace")
        await mkdir(workspace, { recursive: true })
        const created = await client.send(createSessionRequest(workspace, workspace))
        const attached = await client.send(attachToSessionRequest(created.SessionCreated.session.id, "local-auth-drill"))
        await client.subscribeToKernelEvents(created.SessionCreated.session.id, attached.SessionAttached.attachment.id)
        await stopKernel(kernel.child)
        kernel.child = startKernel(kernel, children)
        await waitForKernelIpc(LocalIpcClient, listSessionsRequest, kernel.endpoint, kernel.child, 25_000, kernel.env)
        // Reuse both client lanes after the kernel has rotated its token.
        await client.send(listSessionsRequest())
        await client.subscribeToKernelEvents(created.SessionCreated.session.id, attached.SessionAttached.attachment.id)
        await client.close()
        await stopKernel(kernel.child)
        const logs = kernel.output + await readLogs(kernel.env.CHARIOX_LOG_DIR)
        assert.doesNotMatch(logs, /without the local auth token|wrong local auth token/)
        // First-party lanes above produce no auth refusals. Independently
        // probe raw upgrades after restart so client-side checks cannot mask
        // the kernel's enforcement boundary.
        kernel.child = startKernel(kernel, children)
        await waitForKernelIpc(LocalIpcClient, listSessionsRequest, kernel.endpoint, kernel.child, 25_000, kernel.env)
        for (const headers of [undefined, { authorization: "Bearer wrong-drill-fixture" }]) {
          const rejection = await rejectedUpgrade(kernel.endpoint, headers)
          assert.equal(rejection.status, 401)
          assert.match(rejection.body, /kernel-local-auth\/\d+\.token/)
          assert.ok(rejection.body.includes(`ws+unix://${kernel.env.CHARIOX_DAEMON_SOCKET}`))
          assert.doesNotMatch(rejection.body, /wrong-drill-fixture|chx_kat_/)
        }
        const unix = new LocalIpcClient(`ws+unix://${kernel.env.CHARIOX_DAEMON_SOCKET}`)
        try {
          await assert.rejects(unix.send(listSessionsRequest()), (error) => error.code === "kernel_access_denied")
        } finally {
          unix.destroy()
        }
        await stopKernel(kernel.child)
      }
    } finally {
      for (const client of clients) client.destroy()
      await Promise.all([...children].map(stopKernel))
      await rm(root, { recursive: true, force: true })
    }
  })
}

function startKernel(kernel, children) {
  const child = spawn(process.env.CHARIOX_LOCAL_AUTH_KERNEL_BINARY, [], {
    cwd: kernel.directory, env: kernel.env, stdio: ["ignore", "pipe", "pipe"],
  })
  children.add(child)
  for (const stream of [child.stdout, child.stderr]) {
    stream.on("data", (chunk) => { kernel.output += chunk.toString() })
  }
  return child
}

async function stopKernel(child) {
  if (child.exitCode !== null || child.signalCode !== null) return
  const exited = once(child, "exit")
  child.kill("SIGTERM")
  const timer = setTimeout(() => child.kill("SIGKILL"), 5_000)
  try { await exited } finally { clearTimeout(timer) }
}

async function reservePorts() {
  const servers = []
  try {
    for (let index = 0; index < 4; index++) {
      const server = net.createServer()
      servers.push(server)
      server.listen(0, "127.0.0.1")
      await once(server, "listening")
    }
    return servers.map((server) => server.address().port)
  } finally {
    await Promise.all(servers.map((server) => new Promise((resolve) => server.close(resolve))))
  }
}

async function readLogs(directory) {
  let logs = ""
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const file = path.join(directory, entry.name)
    logs += entry.isDirectory() ? await readLogs(file) : await readFile(file, "utf8")
  }
  return logs
}

async function rejectedUpgrade(endpoint, headers) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(endpoint, { headers })
    const timer = setTimeout(() => { socket.terminate(); reject(new Error("upgrade did not resolve")) }, 5000)
    socket.once("open", () => { clearTimeout(timer); socket.terminate(); reject(new Error("unauthenticated upgrade admitted")) })
    socket.on("error", () => {})
    socket.once("unexpected-response", (_request, response) => {
      let body = ""
      response.setEncoding("utf8")
      response.on("data", (chunk) => { body += chunk })
      response.once("end", () => {
        clearTimeout(timer)
        socket.terminate()
        resolve({ status: response.statusCode, body })
      })
      response.once("error", (error) => { clearTimeout(timer); socket.terminate(); reject(error) })
    })
  })
}
