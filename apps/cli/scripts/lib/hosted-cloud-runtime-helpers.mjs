import { spawn } from "node:child_process"
import { setTimeout as sleep } from "node:timers/promises"

export async function callRuntimeMcp(serverUrl, authToken, method, params = {}, options = {}) {
  const timeoutMs = options.timeoutMs ?? 60_000
  const maxAttempts = options.retryTransient ? (options.maxAttempts ?? 3) : 1
  let lastError = null
  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    const controller = new AbortController()
    const timeout = setTimeout(() => controller.abort(new Error(`runtime MCP ${method} timed out after ${timeoutMs}ms`)), timeoutMs)
    try {
      const response = await fetch(serverUrl, {
        method: "POST",
        headers: {
          Authorization: `Bearer ${authToken}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify({ jsonrpc: "2.0", id: `${Date.now()}-${attempt}`, method, params }),
        signal: controller.signal,
      })
      const json = await response.json()
      if (json.error) throw new Error(`runtime MCP ${method} failed: ${JSON.stringify(json.error)}`)
      return json.result
    } catch (error) {
      lastError = error
      if (attempt >= maxAttempts || !isTransientRuntimeMcpRelayError(error)) throw error
      await sleep(500 * attempt)
    } finally {
      clearTimeout(timeout)
    }
  }
  throw lastError
}

function isTransientRuntimeMcpRelayError(error) {
  const message = String(error?.message ?? error)
  return message.includes("target daemon disconnected from relay")
    || message.includes("read temporary relay peer response")
    || message.includes("timed out waiting for relay peer response")
    || message.includes("relay read failed or ended")
}

export async function expectRuntimeMcpReject(serverUrl, authToken, method, params = {}) {
  try {
    const result = await callRuntimeMcp(serverUrl, authToken, method, params)
    if (result?.isError) return result
    throw new Error(`runtime MCP ${method} unexpectedly succeeded: ${JSON.stringify(result)}`)
  } catch (error) {
    return { error: String(error?.message ?? error) }
  }
}

export async function waitForRuntimeTool(serverUrl, authToken, name, present) {
  let lastTools = null
  for (let attempt = 0; attempt < 240; attempt += 1) {
    const tools = await callRuntimeMcp(serverUrl, authToken, "tools/list")
    lastTools = tools
    const found = tools.tools.some((tool) => tool.name === name)
    if (found === present) return tools
    await sleep(250)
  }
  throw new Error(`tool ${name} did not become ${present ? "advertised" : "revoked"}: ${JSON.stringify(lastTools)}`)
}

export async function runCommand(command, args, cwd) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd, stdio: ["ignore", "pipe", "pipe"] })
    let stdout = ""
    let stderr = ""
    child.stdout.on("data", (chunk) => {
      stdout += chunk.toString()
    })
    child.stderr.on("data", (chunk) => {
      stderr += chunk.toString()
    })
    child.on("error", reject)
    child.on("close", (code, signal) => {
      if (code === 0) {
        resolve({ stdout, stderr })
        return
      }
      reject(new Error(`${command} ${args.join(" ")} failed with code ${code} signal ${signal ?? "none"}: ${stderr || stdout}`))
    })
  })
}
