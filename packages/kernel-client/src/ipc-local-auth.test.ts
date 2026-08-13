import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { mkdtemp, rm, stat, symlink, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { WebSocketServer } from "ws"

const ipcModuleUrl = new URL("./ipc.js", import.meta.url).href
const localAuthEnvironmentNames = [
  "CHARIOX_KERNEL_HOST",
  "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN",
  "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE",
  "CHARIOX_KERNEL_PORT",
  "CHARIOX_KERNEL_URL",
  "CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL",
  "CHARIOX_PUBLICATION_AGENT_APP_AUDIT_URL_FILE",
  "CHARIOX_PUBLICATION_CLOUD_API_URL",
  "CHARIOX_PUBLICATION_CLOUD_DEPLOYMENT_ID",
  "CHARIOX_PUBLICATION_CLOUD_RUNNER_KEY",
] as const

test("LocalIpcClient consumes a private auth file before authenticating local upgrades", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-kernel-local-auth-"))
  const tokenFile = join(root, "token")
  await writeFile(tokenFile, "kernel-local-auth-sentinel", { mode: 0o600 })
  const { server, endpoint, authorizations } = await startKernelServer()
  t.after(async () => {
    await closeKernelServer(server)
    await rm(root, { recursive: true, force: true })
  })

  const result = await runClientScript(`
    consumeKernelLocalAuthTokenFromEnv()
    const client = new LocalIpcClient(${JSON.stringify(endpoint)})
    try {
      const response = await client.send({ ListSessions: null })
      process.stdout.write(JSON.stringify(response))
    } finally {
      client.destroy()
    }
  `, {
    CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE: tokenFile,
    CHARIOX_KERNEL_URL: endpoint,
  })

  assert.equal(result.code, 0, result.stderr)
  assert.deepEqual(JSON.parse(result.stdout), { authenticated: true })
  assert.deepEqual(authorizations, ["Bearer kernel-local-auth-sentinel"])
  await assert.rejects(stat(tokenFile), { code: "ENOENT" })
})

test("LocalIpcClient binds a consumed auth file to one exact kernel endpoint", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-kernel-local-auth-bound-"))
  const tokenFile = join(root, "token")
  await writeFile(tokenFile, "bound-kernel-token", { mode: 0o600 })
  const first = await startKernelServer()
  const second = await startKernelServer()
  t.after(async () => {
    await closeKernelServer(first.server)
    await closeKernelServer(second.server)
    await rm(root, { recursive: true, force: true })
  })

  const result = await runClientScript(`
    async function send(endpoint) {
      const client = new LocalIpcClient(endpoint)
      try {
        await client.send({ ListSessions: null })
      } finally {
        client.destroy()
      }
    }
    await send(${JSON.stringify(first.endpoint)})
    await send(${JSON.stringify(first.endpoint)})
    let rejected = false
    try {
      await send(${JSON.stringify(second.endpoint)})
    } catch (error) {
      rejected = /bound to kernel endpoint/i.test(String(error))
    }
    if (!rejected) throw new Error("consumed credential was not bound to its first endpoint")
  `, {
    CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE: tokenFile,
  })

  assert.equal(result.code, 0, result.stderr)
  assert.deepEqual(first.authorizations, ["Bearer bound-kernel-token", "Bearer bound-kernel-token"])
  assert.deepEqual(second.authorizations, [])
})

test("LocalIpcClient rejects credential use on a non-canonical loopback endpoint", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-kernel-local-auth-endpoint-"))
  const tokenFile = join(root, "token")
  await writeFile(tokenFile, "endpoint-kernel-token", { mode: 0o600 })
  t.after(() => rm(root, { recursive: true, force: true }))

  const result = await runClientScript(`
    let rejected = false
    try {
      new LocalIpcClient("ws://localhost:43118")
    } catch (error) {
      rejected = /canonical loopback kernel endpoint/i.test(String(error))
    }
    if (!rejected) throw new Error("non-canonical endpoint accepted a local credential")
  `, {
    CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE: tokenFile,
  })

  assert.equal(result.code, 0, result.stderr)
  assert.equal((await stat(tokenFile)).isFile(), true)
})

test("LocalIpcClient rejects process-environment tokens in hosted publication gateways", async () => {
  const result = await runClientScript(`
    let rejected = false
    try {
      new LocalIpcClient("ws://127.0.0.1:43118")
    } catch (error) {
      rejected = /hosted publication gateways require.*token file/i.test(String(error))
    }
    if (!rejected) throw new Error("hosted gateway accepted a process-environment token")
  `, {
    CHARIOX_KERNEL_LOCAL_AUTH_TOKEN: "unsafe-process-token",
    CHARIOX_PUBLICATION_CLOUD_DEPLOYMENT_ID: "deployment-hosted",
  })

  assert.equal(result.code, 0, result.stderr)
})

test("LocalIpcClient retains direct-token compatibility outside hosted publication gateways", async (t) => {
  const { server, endpoint, authorizations } = await startKernelServer()
  t.after(() => closeKernelServer(server))

  const result = await runClientScript(`
    const client = new LocalIpcClient(${JSON.stringify(endpoint)})
    try {
      await client.send({ ListSessions: null })
    } finally {
      client.destroy()
    }
  `, {
    CHARIOX_KERNEL_LOCAL_AUTH_TOKEN: "self-host-development-token",
  })

  assert.equal(result.code, 0, result.stderr)
  assert.deepEqual(authorizations, ["Bearer self-host-development-token"])
})

test("LocalIpcClient refuses a symlinked one-shot credential without consuming its target", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-kernel-local-auth-symlink-"))
  const target = join(root, "target")
  const tokenFile = join(root, "token")
  await writeFile(target, "symlink-target-token", { mode: 0o600 })
  await symlink(target, tokenFile)
  t.after(() => rm(root, { recursive: true, force: true }))

  const result = await runClientScript(`
    let rejected = false
    try {
      new LocalIpcClient("ws://127.0.0.1:43118")
    } catch (error) {
      rejected = /kernel local auth token file|too many levels of symbolic links/i.test(String(error))
    }
    if (!rejected) throw new Error("symlinked credential was accepted")
  `, {
    CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE: tokenFile,
  })

  assert.equal(result.code, 0, result.stderr)
  assert.equal((await stat(target)).isFile(), true)
})

test("LocalIpcClient rejects oversized one-shot credentials without consuming them", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "chariox-kernel-local-auth-oversized-"))
  const tokenFile = join(root, "token")
  await writeFile(tokenFile, "x".repeat(8 * 1024 + 1), { mode: 0o600 })
  t.after(() => rm(root, { recursive: true, force: true }))

  const result = await runClientScript(`
    let rejected = false
    try {
      new LocalIpcClient("ws://127.0.0.1:43118")
    } catch (error) {
      rejected = /bounded.*mode 0600/i.test(String(error))
    }
    if (!rejected) throw new Error("oversized credential was accepted")
  `, {
    CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE: tokenFile,
  })

  assert.equal(result.code, 0, result.stderr)
  assert.equal((await stat(tokenFile)).isFile(), true)
})

async function startKernelServer() {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 })
  await new Promise<void>((resolve) => server.once("listening", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  const authorizations: Array<string | undefined> = []
  server.on("connection", (socket, request) => {
    authorizations.push(request.headers.authorization)
    socket.once("message", (payload) => {
      const frame = JSON.parse(String(payload)) as { request_id: string }
      socket.send(JSON.stringify({
        type: "response",
        request_id: frame.request_id,
        response: { authenticated: true },
        error: null,
      }))
    })
  })
  return {
    server,
    endpoint: `ws://127.0.0.1:${address.port}`,
    authorizations,
  }
}

async function closeKernelServer(server: WebSocketServer) {
  for (const socket of server.clients) socket.terminate()
  await new Promise<void>((resolve) => server.close(() => resolve()))
}

async function runClientScript(script: string, additions: NodeJS.ProcessEnv) {
  const environment = { ...process.env }
  for (const name of localAuthEnvironmentNames) delete environment[name]
  Object.assign(environment, additions)
  const child = spawn(process.execPath, [
    "--input-type=module",
    "--eval",
    `import { LocalIpcClient, consumeKernelLocalAuthTokenFromEnv } from ${JSON.stringify(ipcModuleUrl)};\n${script}`,
  ], {
    env: environment,
    stdio: ["ignore", "pipe", "pipe"],
  })
  let stdout = ""
  let stderr = ""
  child.stdout.setEncoding("utf8")
  child.stderr.setEncoding("utf8")
  child.stdout.on("data", (chunk: string) => { stdout += chunk })
  child.stderr.on("data", (chunk: string) => { stderr += chunk })
  const code = await new Promise<number | null>((resolve, reject) => {
    child.once("error", reject)
    child.once("exit", resolve)
  })
  return { code, stdout, stderr }
}
