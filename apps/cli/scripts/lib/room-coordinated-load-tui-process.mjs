import { randomUUID } from "node:crypto"
import { execFile } from "node:child_process"
import { createConnection } from "node:net"
import { unlink } from "node:fs/promises"
import path from "node:path"
import { performance } from "node:perf_hooks"
import { promisify } from "node:util"

const execFileAsync = promisify(execFile)
const processCommandTimeoutMs = 10_000
const maximumCommandOutputBytes = 2 * 1024 * 1024

export async function readAutomationSnapshot(socketPath, signal) {
  const id = `${process.pid}-${randomUUID()}`
  return await new Promise((resolve, reject) => {
    const socket = createConnection(socketPath)
    socket.setEncoding("utf8")
    let buffer = ""
    let settled = false
    const finish = (error, value) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      signal?.removeEventListener("abort", abort)
      socket.destroy()
      if (error) reject(error)
      else resolve(value)
    }
    const abort = () => finish(new Error("automation snapshot aborted"))
    const timer = setTimeout(() => finish(new Error("automation snapshot timed out")), 3_000)
    signal?.addEventListener("abort", abort, { once: true })
    socket.once("connect", () => socket.write(`${JSON.stringify({ id, action: "snapshot" })}\n`))
    socket.on("data", (chunk) => {
      buffer += chunk
      if (buffer.length > 256 * 1024) {
        finish(new Error("automation snapshot exceeded its byte limit"))
        return
      }
      const newline = buffer.indexOf("\n")
      if (newline < 0) return
      try {
        const response = JSON.parse(buffer.slice(0, newline))
        if (response.id !== id || response.ok !== true) finish(new Error("automation snapshot failed"))
        else finish(null, response.data)
      } catch {
        finish(new Error("automation snapshot was malformed"))
      }
    })
    socket.once("error", () => finish(new Error("automation socket unavailable")))
  })
}

export async function waitForAutomationRoom(state, roomId, signal) {
  const deadline = performance.now() + 20_000
  while (performance.now() < deadline) {
    if (state.startupError) throw new Error("owned TUI could not start")
    if (state.child.exitCode !== null || state.child.signalCode !== null) {
      throw new Error("owned TUI exited before Room attachment")
    }
    try {
      const snapshot = await readAutomationSnapshot(state.automationSocket, signal)
      if (snapshot?.session?.id !== roomId || typeof snapshot.attachmentId !== "string" || !snapshot.attachmentId) {
        throw new Error("TUI attached to a different Room or omitted attachment identity")
      }
      state.attachmentId = snapshot.attachmentId
      return
    } catch (error) {
      if (error.message.includes("different Room")) throw error
      await sleep(100, signal)
    }
  }
  throw new Error("owned TUI did not attach within its startup bound")
}

export function validateTuiSnapshot(snapshot, roomId, attachmentId) {
  if (snapshot?.session?.id !== roomId || snapshot.attachmentId !== attachmentId) {
    throw new Error("normal TUI client changed Room or attachment identity")
  }
}

export async function readOwnedProcessMetrics(ownedTasks) {
  const groupIds = ownedTasks.filter((task) => task.kind === "tui")
    .map((task) => task.processGroupId)
    .filter(Number.isSafeInteger)
  const pids = await currentOwnedPids(ownedTasks)
  if (!pids.length) return { count: 0, rssBytes: 0 }
  const { stdout } = await execFileAsync("ps", ["-o", "pid=,rss=", "-p", pids.join(",")], {
    timeout: processCommandTimeoutMs,
    maxBuffer: maximumCommandOutputBytes,
  })
  const metrics = stdout.split("\n").filter(Boolean).map((line) => {
    const [pid, rssKb] = line.trim().split(/\s+/).map(Number)
    return { pid, rssBytes: rssKb * 1024 }
  }).filter((row) => pids.includes(row.pid))
  return { count: metrics.length, rssBytes: metrics.reduce((total, row) => total + row.rssBytes, 0), groupCount: groupIds.length }
}

async function ownedProcessRows(tasks) {
  const groups = tasks.filter((task) => task.kind === "tui")
    .map((task) => task.processGroupId).filter(Number.isSafeInteger)
  if (!groups.length) return []
  const { stdout } = await execFileAsync("ps", ["-axo", "pid=,ppid=,pgid="], {
    timeout: processCommandTimeoutMs, maxBuffer: maximumCommandOutputBytes,
  })
  const rows = stdout.split("\n").map((line) => line.trim().split(/\s+/).map(Number))
    .filter((row) => row.length === 3)
  const owned = new Set(rows.filter((row) => groups.includes(row[2])).map((row) => row[0]))
  // Linux script gives its shell/TUI children separate process groups.
  let changed = true
  while (changed) {
    changed = false
    for (const [pid, parent] of rows) {
      if (owned.has(parent) && !owned.has(pid)) { owned.add(pid); changed = true }
    }
  }
  return rows.filter(([pid]) => owned.has(pid))
}

export async function currentOwnedPids(tasks) {
  return (await ownedProcessRows(tasks)).map(([pid]) => pid).sort((a, b) => a - b)
}

export async function stopProcessGroup(groupId, child) {
  if (!Number.isSafeInteger(groupId) || groupId <= 0) throw new Error("owned TUI process-group identity is invalid")
  const owned = await ownedProcessRows([{ kind: "tui", processGroupId: groupId }])
  const groups = [...new Set([groupId, ...owned.map(([, , group]) => group)])]
  const tasks = groups.map((processGroupId) => ({ kind: "tui", processGroupId }))
  const kill = (signal) => {
    for (const group of groups) {
      try { process.kill(-group, signal) } catch (error) { if (error.code !== "ESRCH") throw error }
    }
  }
  kill("SIGTERM")
  if (child.exitCode === null && child.signalCode === null) {
    await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(2_000)])
  }
  if ((await currentOwnedPids(tasks)).length) {
    kill("SIGKILL")
    await sleep(250)
  }
  if ((await currentOwnedPids(tasks)).length) throw new Error("owned TUI process group remained after cleanup")
}

export async function removeAutomationSocket(socketPath) {
  await unlink(socketPath).catch((error) => { if (error.code !== "ENOENT") throw error })
}

export function tuiEnvironment(home, relayToken, tokenEnv, localKernelAuthEnvironment) {
  const env = {
    PATH: process.env.PATH ?? "/usr/bin:/bin",
    HOME: home,
    CHARIOX_HOME: path.join(home, ".chariox"),
    XDG_CONFIG_HOME: path.join(home, ".config"),
    XDG_STATE_HOME: path.join(home, ".local", "state"),
    XDG_CACHE_HOME: path.join(home, ".cache"),
    TERM: process.env.TERM ?? "xterm-256color",
    LANG: process.env.LANG ?? "C.UTF-8",
  }
  if (relayToken) env[tokenEnv] = relayToken
  Object.assign(env, localKernelAuthEnvironment)
  return env
}

export async function sleep(milliseconds, signal) {
  if (signal?.aborted) throw new Error("coordinated load interrupted")
  await new Promise((resolve, reject) => {
    const timer = setTimeout(done, milliseconds)
    function done() {
      signal?.removeEventListener("abort", abort)
      resolve()
    }
    function abort() {
      clearTimeout(timer)
      signal?.removeEventListener("abort", abort)
      reject(new Error("coordinated load interrupted"))
    }
    signal?.addEventListener("abort", abort, { once: true })
  })
}
