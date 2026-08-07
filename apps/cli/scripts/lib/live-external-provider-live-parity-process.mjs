import net from "node:net"
import { writeFile } from "node:fs/promises"
import { setTimeout as sleep } from "node:timers/promises"

export async function waitForAutomation(socketPath, child) {
  const deadline = Date.now() + 90_000
  let lastError = null
  while (Date.now() < deadline) {
    if (child.exitCode != null) throw new Error(`TUI exited before automation socket became ready: ${child.exitCode}`)
    try {
      await automationRequest(socketPath, { action: "ping" }, 5_000)
      return
    } catch (error) {
      lastError = error
      await sleep(250)
    }
  }
  throw new Error(`automation socket did not become ready: ${lastError?.message ?? lastError}`)
}

export async function automationRequest(socketPath, request, timeoutMs = 20_000) {
  return await new Promise((resolve, reject) => {
    const socket = net.createConnection(socketPath)
    let buffer = ""
    socket.setTimeout(timeoutMs)
    socket.once("error", reject)
    socket.once("timeout", () => reject(new Error(`automation request timed out: ${JSON.stringify(request)}`)))
    socket.on("data", (chunk) => {
      buffer += chunk.toString("utf8")
      const index = buffer.indexOf("\n")
      if (index < 0) return
      const line = buffer.slice(0, index)
      socket.end()
      const response = JSON.parse(line)
      if (!response.ok) reject(new Error(response.error ?? "automation request failed"))
      else resolve(response.data)
    })
    socket.once("connect", () => {
      socket.write(`${JSON.stringify({ id: Date.now(), ...request })}\n`)
    })
  })
}

export function pipeChildLogs(child, stdoutPath, stderrPath) {
  let stdout = ""
  let stderr = ""
  child.stdout?.on("data", (chunk) => {
    stdout += chunk.toString("utf8")
    if (stdout.length > 250_000) stdout = stdout.slice(-250_000)
    void writeFile(stdoutPath, stdout, "utf8").catch(() => {})
  })
  child.stderr?.on("data", (chunk) => {
    stderr += chunk.toString("utf8")
    if (stderr.length > 250_000) stderr = stderr.slice(-250_000)
    void writeFile(stderrPath, stderr, "utf8").catch(() => {})
  })
}

export async function closeWithTimeout(target, label, timeoutMs = 3_000) {
  if (!target?.close) return
  let timedOut = false
  let timeoutId
  try {
    await Promise.race([
      Promise.resolve().then(() => target.close()),
      new Promise((resolve) => {
        timeoutId = setTimeout(() => {
          timedOut = true
          console.warn(`${label} close timed out`)
          resolve()
        }, timeoutMs)
      }),
    ])
  } catch {
    // Cleanup is best-effort.
  } finally {
    clearTimeout(timeoutId)
  }
  if (timedOut && typeof target.process === "function") {
    target.process()?.kill("SIGKILL")
  }
}

export function stopChild(child) {
  if (!child || child.exitCode != null) return
  child.kill("SIGTERM")
  setTimeout(() => {
    if (child.exitCode == null) child.kill("SIGKILL")
  }, 2_000).unref()
}
