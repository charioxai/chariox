import { execFile, spawn } from "node:child_process"
import net from "node:net"
import { access, readFile } from "node:fs/promises"
import { promisify } from "node:util"
import { setTimeout as sleep } from "node:timers/promises"

const execFileAsync = promisify(execFile)

export function makePorts(base = 52000 + Math.floor(Math.random() * 4000)) {
  return {
    relayPort: base,
    kernelPort: base + 1000,
    workerKernelPort: base + 1100,
    workerMcpPort: base + 1101,
    mcpPort: base + 1001,
    openCodePort: base + 2000,
    codexPort: base + 2001,
  }
}

export async function makeAvailablePorts() {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const ports = makePorts()
    if (await portsAreAvailable(ports)) return ports
  }
  throw new Error("could not find available drill ports")
}

export async function portsAreAvailable(ports) {
  const candidates = [
    ports.relayPort,
    ports.kernelPort,
    ports.workerKernelPort,
    ports.workerMcpPort,
    ports.mcpPort,
    ports.openCodePort,
    ports.codexPort,
  ]
  for (const port of candidates) {
    if (!(await portIsAvailable(port))) return false
  }
  return true
}

export async function portIsAvailable(port) {
  return await new Promise((resolve) => {
    const server = net.createServer()
    server.once("error", () => resolve(false))
    server.listen(port, "127.0.0.1", () => {
      server.close(() => resolve(true))
    })
  })
}

export async function assertBinary(binaryPath, manifestPath, binName) {
  try {
    await access(binaryPath)
  } catch {
    throw new Error(`missing built binary ${binaryPath}; run cargo build --manifest-path ${manifestPath} --bin ${binName} first`)
  }
}

export async function waitForTcpPort(port, host = "127.0.0.1", timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const ready = await new Promise((resolve) => {
      const socket = net.connect({ host, port })
      socket.once("connect", () => {
        socket.destroy()
        resolve(true)
      })
      socket.once("error", () => {
        socket.destroy()
        resolve(false)
      })
    })
    if (ready) return
    await sleep(100)
  }
  throw new Error(`TCP listener ${host}:${port} did not become reachable`)
}

export async function terminateChild(child, signal = "SIGTERM") {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([
    new Promise((resolve) => child.once("exit", resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null) {
    child.kill("SIGKILL")
    await Promise.race([
      new Promise((resolve) => child.once("exit", resolve)),
      sleep(2_000),
    ])
  }
}

export async function runLogged(command, args, options = {}) {
  await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd,
      env: options.env ?? process.env,
      stdio: "inherit",
    })
    child.once("error", reject)
    child.once("exit", (code, signal) => {
      if (code === 0) {
        resolve()
        return
      }
      reject(new Error(`${command} ${args.join(" ")} exited with ${signal ?? code}`))
    })
  })
}

export async function resolveCommandPath(command) {
  const { stdout } = await execFileAsync("bash", ["-lc", `command -v ${shellQuote(command)}`])
  return stdout.trim()
}

export async function screen(name, args) {
  await execFileAsync("screen", ["-S", name, ...args])
}

export async function screenQuit(name) {
  await screen(name, ["-X", "quit"]).catch(() => {})
}

export async function screenStuff(name, text) {
  await screen(name, ["-p", "0", "-X", "stuff", text])
}

export function startScreen(name, logDir, command, args, env) {
  return execFileAsync("screen", [
    "-dmS",
    name,
    "-L",
    command,
    ...args,
  ], { env, cwd: logDir })
}

export async function waitForFileMatch(file, pattern, timeoutMs = 90_000) {
  const deadline = Date.now() + timeoutMs
  let text = ""
  while (Date.now() < deadline) {
    text = await readFile(file, "utf8").catch(() => "")
    const match = text.match(pattern)
    if (match) return { match, text }
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${pattern} in ${file}\n${text.slice(-4000)}`)
}

export async function waitForLogOccurrences(logFile, needle, count, timeoutMs = 120_000) {
  const deadline = Date.now() + timeoutMs
  let text = ""
  while (Date.now() < deadline) {
    text = await readFile(logFile, "utf8").catch(() => "")
    const occurrences = text.split(needle).length - 1
    if (occurrences >= count) return text
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${count} occurrences of ${needle} in ${logFile}\n${text.slice(-4000)}`)
}

export async function waitForCondition({
  label,
  timeoutMs,
  pollMs = 250,
  observe,
  isReady = Boolean,
  describe = stringifyDiagnostic,
  retryOnError = true,
}) {
  const deadline = Date.now() + timeoutMs
  let lastObservation
  let lastError = null
  while (Date.now() < deadline) {
    try {
      lastObservation = await observe()
      if (await isReady(lastObservation)) return lastObservation
      lastError = null
    } catch (error) {
      if (!retryOnError) throw error
      lastError = error
    }
    await sleep(pollMs)
  }
  const details = [
    `timed out waiting for ${label}`,
    lastObservation !== undefined ? `last_observation=${describe(lastObservation)}` : null,
    lastError ? `last_error=${lastError.stack ?? lastError.message ?? String(lastError)}` : null,
  ].filter(Boolean)
  throw new Error(details.join("\n"))
}

function stringifyDiagnostic(value) {
  if (typeof value === "string") return value
  return JSON.stringify(value, null, 2)
}

function shellQuote(value) {
  return `'${String(value).replaceAll("'", "'\\''")}'`
}
