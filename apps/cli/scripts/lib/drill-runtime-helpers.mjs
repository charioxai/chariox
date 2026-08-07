import { execFile, spawn } from "node:child_process"
import { statSync } from "node:fs"
import net from "node:net"
import { access, readFile } from "node:fs/promises"
import path from "node:path"
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

export function makeNonEphemeralDrillPorts(base = 20000 + Math.floor(Math.random() * 4000)) {
  const ports = makePorts(base)
  if (Math.max(...Object.values(ports)) >= 32768) {
    throw new Error("non-ephemeral drill ports must stay below 32768")
  }
  return ports
}

export function withDevStubProviderInventory(env) {
  return {
    ...env,
    ARROBA_PROVIDER_DEV_STUB: "1",
  }
}

export async function makeAvailablePorts({
  candidateFactory = makePorts,
  localAvailability = portsAreAvailable,
  additionalAvailability = async () => true,
  maxAttempts = 80,
} = {}) {
  for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
    const ports = candidateFactory()
    if (!(await localAvailability(ports))) continue
    if (await additionalAvailability(ports)) return ports
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

export async function resolveBuiltBinary(binaryPath, manifestPath, binName) {
  const resolved = newestBuiltBinary(binaryPath, manifestPath, binName)
  if (resolved) return resolved
  await access(binaryPath)
  return binaryPath
}

export function resolveBuiltBinarySync(binaryPath, manifestPath, binName) {
  return newestBuiltBinary(binaryPath, manifestPath, binName) ?? binaryPath
}

function newestBuiltBinary(binaryPath, manifestPath, binName) {
  const workspaceBinaryPath = path.join(
    path.dirname(path.dirname(path.dirname(manifestPath))),
    "target",
    "debug",
    binName,
  )
  let newest = null
  for (const candidate of new Set([binaryPath, workspaceBinaryPath])) {
    try {
      const modifiedAtMs = statSync(candidate).mtimeMs
      if (!newest || modifiedAtMs > newest.modifiedAtMs) {
        newest = { path: candidate, modifiedAtMs }
      }
    } catch {}
  }
  return newest?.path ?? null
}

export async function waitForTcpPort(port, host = "127.0.0.1", timeoutMs = 15_000) {
  await waitForCondition({
    label: `TCP listener ${host}:${port}`,
    timeoutMs,
    pollMs: 100,
    observe: async () => await new Promise((resolve) => {
      const socket = net.connect({ host, port })
      socket.once("connect", () => {
        socket.destroy()
        resolve({ host, port, reachable: true })
      })
      socket.once("error", (error) => {
        socket.destroy()
        resolve({ host, port, reachable: false, error: error.message })
      })
    }),
    isReady: (observation) => observation.reachable,
  })
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

export function findMatchingProcessIdsFromPsOutput(psOutput, patterns, currentPid = process.pid) {
  const matchers = processMatchPatterns(patterns)
  const matches = []
  const seen = new Set()
  for (const line of psOutput.split(/\r?\n/)) {
    const match = line.match(/^\s*(\d+)\s+(.*)$/)
    if (!match) continue
    const pid = Number(match[1])
    if (!Number.isSafeInteger(pid) || pid <= 0 || pid === currentPid) continue
    const command = match[2]
    if (!matchers.some((matcher) => matcher(command))) continue
    if (seen.has(pid)) continue
    seen.add(pid)
    matches.push(pid)
  }
  return matches
}

export async function terminateMatchingProcesses(patterns, options = {}) {
  const currentPid = options.currentPid ?? process.pid
  const graceMs = options.graceMs ?? 3_000
  const pollMs = options.pollMs ?? 100
  const signal = options.signal ?? "SIGTERM"
  const killSignal = options.killSignal ?? "SIGKILL"
  const pids = await matchingProcessIds(patterns, currentPid)
  for (const pid of pids) {
    sendProcessSignal(pid, signal)
  }
  const survivors = await waitForProcessExit(pids, graceMs, pollMs)
  for (const pid of survivors) {
    sendProcessSignal(pid, killSignal)
  }
  const killed = await waitForProcessExit(survivors, options.killGraceMs ?? 1_000, pollMs)
  return {
    signaled: pids,
    killed: survivors.filter((pid) => !killed.includes(pid)),
    remaining: killed,
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
      reject(new Error(`${formatDrillCommandLine(command, args)} exited with ${signal ?? code}`))
    })
  })
}

export function formatDrillCommandLine(command, args = []) {
  return [command, ...args].map(shellDisplayArg).join(" ")
}

export async function resolveCommandPath(command) {
  const { stdout } = await execFileAsync("bash", ["-lc", `command -v ${shellQuote(command)}`])
  return stdout.trim()
}

export function providerAuthFailureFromTerminalText(value) {
  const normalized = String(value ?? "")
    .replace(/\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07]*(?:\x07|\x1b\\))/g, " ")
    .replace(/\s+/g, " ")
    .trim()
  const match = normalized.match(/\b(?:login expired|not logged in|login required|token refresh failed|authentication failed|unauthori[sz]ed)\b/i)
  return match?.[0] ?? null
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

export function screenSessionListContains(output, name) {
  return String(output ?? "").split(/\r?\n/).some((line) => {
    const session = line.trim().split(/\s+/, 1)[0] ?? ""
    const separator = session.indexOf(".")
    return separator > 0 && session.slice(separator + 1) === name
  })
}

export async function screenIsRunning(name) {
  try {
    const { stdout } = await execFileAsync("screen", ["-ls"])
    return screenSessionListContains(stdout, name)
  } catch (error) {
    const stdout = typeof error === "object" && error && "stdout" in error ? error.stdout : ""
    return screenSessionListContains(stdout, name)
  }
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

export async function waitForScreenMatch(screenName, hardcopyFile, pattern, timeoutMs = 90_000) {
  const deadline = Date.now() + timeoutMs
  let text = ""
  let lastError = null
  while (Date.now() < deadline) {
    try {
      await screen(screenName, ["-p", "0", "-X", "hardcopy", "-h", hardcopyFile])
      text = await readFile(hardcopyFile, "utf8")
      const match = text.match(pattern)
      if (match) return { match, text }
      lastError = null
    } catch (error) {
      lastError = error
    }
    await sleep(250)
  }
  const errorDetail = lastError ? `\nlast_error=${lastError}` : ""
  throw new Error(
    `timed out waiting for rendered ${pattern} in screen ${screenName}${errorDetail}\n${text.slice(-4000)}`,
  )
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

async function matchingProcessIds(patterns, currentPid) {
  const { stdout } = await execFileAsync("ps", ["-axo", "pid=,command="], {
    maxBuffer: 16 * 1024 * 1024,
  })
  return findMatchingProcessIdsFromPsOutput(stdout, patterns, currentPid)
}

function processMatchPatterns(patterns) {
  return patterns
    .filter((pattern) => pattern !== null && pattern !== undefined && pattern !== "")
    .map((pattern) => {
      if (typeof pattern === "string") {
        return (command) => command.includes(pattern)
      }
      if (pattern instanceof RegExp) {
        return (command) => {
          pattern.lastIndex = 0
          return pattern.test(command)
        }
      }
      if (typeof pattern === "function") return pattern
      throw new TypeError(`unsupported process match pattern: ${String(pattern)}`)
    })
}

function processIsAlive(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    return error?.code === "EPERM"
  }
}

function sendProcessSignal(pid, signal) {
  try {
    process.kill(pid, signal)
  } catch (error) {
    if (error?.code !== "ESRCH") throw error
  }
}

async function waitForProcessExit(pids, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let survivors = pids.filter(processIsAlive)
  while (survivors.length > 0 && Date.now() < deadline) {
    await sleep(pollMs)
    survivors = pids.filter(processIsAlive)
  }
  return survivors
}

function shellDisplayArg(value) {
  const arg = String(value)
  if (/^[A-Za-z0-9_./:@=,+%-]+$/.test(arg)) return arg
  return shellQuote(arg)
}

function shellQuote(value) {
  return `'${String(value).replaceAll("'", "'\\''")}'`
}
