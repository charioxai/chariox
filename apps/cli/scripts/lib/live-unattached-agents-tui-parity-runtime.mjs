import { spawn } from "node:child_process"
import { mkdir, rm, writeFile } from "node:fs/promises"
import net from "node:net"
import path from "node:path"

export function createUnattachedAgentsTuiParityDrillRuntime({ repoRoot, cliRoot }) {
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

  function nowStamp() {
    return new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z")
  }

  async function reserveFreePort() {
    return await new Promise((resolve, reject) => {
      const server = net.createServer()
      server.once("error", reject)
      server.listen(0, "127.0.0.1", () => {
        const address = server.address()
        const port = typeof address === "object" && address ? address.port : null
        server.close((error) => {
          if (error) {
            reject(error)
            return
          }
          if (!port) {
            reject(new Error("failed to reserve free port"))
            return
          }
          resolve(port)
        })
      })
    })
  }

  async function makePorts() {
    const ports = new Set()
    while (ports.size < 4) {
      ports.add(await reserveFreePort())
    }
    const [kernelPort, mcpPort, opencodePort, codexPort] = [...ports]
    return {
      kernelPort,
      mcpPort,
      opencodePort,
      codexPort,
    }
  }

  async function run(command, args, options = {}) {
    return await new Promise((resolve, reject) => {
      const child = spawn(command, args, {
        cwd: options.cwd ?? repoRoot,
        env: options.env ?? process.env,
        stdio: ["ignore", "pipe", "pipe"],
      })
      let stdout = ""
      let stderr = ""
      child.stdout.on("data", (chunk) => { stdout += chunk.toString() })
      child.stderr.on("data", (chunk) => { stderr += chunk.toString() })
      child.on("error", reject)
      child.on("close", (code, signal) => resolve({ code, signal, stdout, stderr }))
    })
  }

  async function ensureBuilt() {
    const kernel = await run("cargo", ["build", "--manifest-path", path.join(repoRoot, "apps/kernel/Cargo.toml"), "--bin", "chariox-kernel"])
    if (kernel.code !== 0) throw new Error(`kernel build failed\n${kernel.stdout}\n${kernel.stderr}`)
    const kernelClient = await run("pnpm", ["--filter", "@chariox/kernel-client", "run", "build"])
    if (kernelClient.code !== 0) throw new Error(`kernel-client build failed\n${kernelClient.stdout}\n${kernelClient.stderr}`)
    const toolDisplay = await run("pnpm", ["--filter", "@chariox/tool-display", "run", "build"])
    if (toolDisplay.code !== 0) throw new Error(`tool-display build failed\n${toolDisplay.stdout}\n${toolDisplay.stderr}`)
    const cli = await run("node", [path.join(cliRoot, "scripts/build.mjs")])
    if (cli.code !== 0) throw new Error(`cli build failed\n${cli.stdout}\n${cli.stderr}`)
    return {
      kernelBinary: path.join(repoRoot, "apps/kernel/target/debug/chariox-kernel"),
      cliDist: path.join(cliRoot, "dist/index.js"),
    }
  }

  async function waitForKernel(LocalIpcClient, listSessionsRequest, kernelUrl) {
    const deadline = Date.now() + 20_000
    let lastError = null
    while (Date.now() < deadline) {
      const client = new LocalIpcClient(kernelUrl)
      try {
        await client.send(listSessionsRequest())
        await client.close().catch(() => {})
        return
      } catch (error) {
        lastError = error
        await client.close().catch(() => {})
        await sleep(250)
      }
    }
    throw new Error(`kernel did not become ready: ${lastError?.message ?? lastError}`)
  }

  async function waitForSocket(socketPath) {
    const deadline = Date.now() + 20_000
    let lastError = null
    while (Date.now() < deadline) {
      try {
        const socket = net.createConnection(socketPath)
        await new Promise((resolve, reject) => {
          socket.once("connect", resolve)
          socket.once("error", reject)
        })
        socket.destroy()
        return
      } catch (error) {
        lastError = error
        await sleep(250)
      }
    }
    throw new Error(`automation socket did not become ready: ${lastError?.message ?? lastError}`)
  }

  function createAutomationClient(socketPath) {
    const socket = net.createConnection(socketPath)
    socket.setEncoding("utf8")
    let nextId = 1
    let buffer = ""
    const pending = new Map()
    socket.on("data", (chunk) => {
      buffer += chunk
      while (buffer.includes("\n")) {
        const newline = buffer.indexOf("\n")
        const line = buffer.slice(0, newline).trim()
        buffer = buffer.slice(newline + 1)
        if (!line) continue
        const response = JSON.parse(line)
        const deferred = pending.get(response.id)
        if (!deferred) continue
        pending.delete(response.id)
        if (response.ok) deferred.resolve(response.data)
        else deferred.reject(new Error(response.error ?? "automation command failed"))
      }
    })
    socket.on("error", (error) => {
      for (const deferred of pending.values()) deferred.reject(error)
      pending.clear()
    })
    return {
      send(action, fields = {}) {
        const id = nextId++
        const request = { id, action, ...fields }
        return new Promise((resolve, reject) => {
          pending.set(id, { resolve, reject })
          socket.write(`${JSON.stringify(request)}\n`)
        })
      },
      close() {
        socket.destroy()
      },
    }
  }

  async function waitForSnapshot(automation, predicate, label, timeoutMs = 30_000) {
    const deadline = Date.now() + timeoutMs
    let last = null
    while (Date.now() < deadline) {
      last = await automation.send("snapshot")
      if (predicate(last)) return last
      await sleep(250)
    }
    throw new Error(`timed out waiting for ${label}\nlast snapshot:\n${JSON.stringify(last, null, 2)}`)
  }

  async function waitForCondition(predicate, label, timeoutMs = 30_000) {
    const deadline = Date.now() + timeoutMs
    let last = null
    while (Date.now() < deadline) {
      last = await predicate()
      if (last) return last
      await sleep(250)
    }
    throw new Error(`timed out waiting for ${label}\nlast value:\n${JSON.stringify(last, null, 2)}`)
  }

  async function stopChild(child) {
    if (!child || child.exitCode !== null || child.signalCode !== null) return
    child.kill("SIGTERM")
    const exited = await Promise.race([
      new Promise((resolve) => child.once("exit", () => resolve(true))),
      sleep(5_000).then(() => false),
    ])
    if (!exited && child.exitCode === null && child.signalCode === null) {
      child.kill("SIGKILL")
      await Promise.race([
        new Promise((resolve) => child.once("exit", resolve)),
        sleep(2_000),
      ])
    }
  }

  function unwrap(response, ...keys) {
    for (const key of keys) {
      if (response?.[key]) return response[key]
    }
    return response
  }

  function refreshExternalProviderSessionsRequest(provider = null) {
    return { RefreshExternalProviderSessions: { provider } }
  }

  async function renderTerminalScreenshot(artifactsDir, fileName, title, lines) {
    await mkdir(artifactsDir, { recursive: true })
    const width = 1280
    const height = Math.max(280, 104 + lines.length * 28)
    const svgPath = path.join(artifactsDir, `${fileName}.svg`)
    const pngPath = path.join(artifactsDir, fileName)
    const escaped = (value) => String(value)
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
    const body = lines.map((line, index) => (
      `<text x="48" y="${116 + index * 28}" fill="${line.startsWith("PASS") ? "#8ef0a5" : "#d9e2ec"}" font-size="20">${escaped(line)}</text>`
    )).join("\n")
    await writeFile(svgPath, `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}">
<rect width="100%" height="100%" fill="#101820"/>
<rect x="28" y="28" width="${width - 56}" height="${height - 56}" rx="8" fill="#141f2b" stroke="#3b5269"/>
<text x="48" y="72" fill="#ffffff" font-family="Menlo, Consolas, monospace" font-size="24" font-weight="700">${escaped(title)}</text>
<g font-family="Menlo, Consolas, monospace">${body}</g>
</svg>`, "utf8")
    const result = await run("sips", ["-s", "format", "png", svgPath, "--out", pngPath])
    if (result.code !== 0) throw new Error(`failed to render screenshot ${fileName}: ${result.stdout}\n${result.stderr}`)
    await rm(svgPath, { force: true })
    return pngPath
  }

  return {
    createAutomationClient,
    ensureBuilt,
    makePorts,
    nowStamp,
    refreshExternalProviderSessionsRequest,
    renderTerminalScreenshot,
    stopChild,
    unwrap,
    waitForCondition,
    waitForKernel,
    waitForSnapshot,
    waitForSocket,
  }
}
