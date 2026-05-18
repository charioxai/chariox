import { spawn } from "node:child_process"
import { writeFile } from "node:fs/promises"
import { setTimeout as sleep } from "node:timers/promises"

import WebSocket from "ws"

import { sendTerminalInputRequest } from "../../dist/ipc-requests.js"

export async function runNativeOpenCodePrompt(proxyUrl, providerSessionId, worktree, prompt, logFile, filePath = null) {
  const executable = process.env.ARROBA_OPENCODE_BIN?.trim() || "opencode"
  const args = [
    "run",
    "--attach",
    proxyUrl,
    "--session",
    providerSessionId,
    "--dir",
    worktree,
  ]
  if (filePath) args.push("--file", filePath, "--")
  args.push(prompt)
  const output = await new Promise((resolve, reject) => {
    const child = spawn(executable, args, {
      cwd: worktree,
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    const timer = setTimeout(() => {
      child.kill("SIGTERM")
      reject(new Error(`opencode run --attach timed out for ${providerSessionId}`))
    }, 180_000)
    child.stdout?.on("data", (chunk) => {
      stdout += chunk.toString("utf8")
    })
    child.stderr?.on("data", (chunk) => {
      stderr += chunk.toString("utf8")
    })
    child.once("error", (error) => {
      clearTimeout(timer)
      reject(error)
    })
    child.once("exit", (code, signal) => {
      clearTimeout(timer)
      if (code === 0) {
        resolve(`${stdout}\n${stderr}`)
        return
      }
      reject(new Error(`opencode run --attach exited with ${signal ?? code}\n${stdout}\n${stderr}`))
    })
  })
  await writeFile(logFile, output)
}

export async function runNativeOpenCodePromptDetached(proxyUrl, providerSessionId, worktree, prompt) {
  const executable = process.env.ARROBA_OPENCODE_BIN?.trim() || "opencode"
  const child = spawn(executable, [
    "run",
    "--attach",
    proxyUrl,
    "--session",
    providerSessionId,
    "--dir",
    worktree,
    prompt,
  ], {
    cwd: worktree,
    env: process.env,
    stdio: ["ignore", "pipe", "pipe"],
  })
  let stdout = ""
  let stderr = ""
  let exitResult = null
  let exitError = null
  child.stdout?.on("data", (chunk) => {
    stdout += chunk.toString("utf8")
  })
  child.stderr?.on("data", (chunk) => {
    stderr += chunk.toString("utf8")
  })
  child.once("error", (error) => {
    exitError = error
  })
  child.once("exit", (code, signal) => {
    exitResult = { code, signal }
  })
  return {
    wait: async (timeoutMs = 240_000) => await new Promise((resolve, reject) => {
      if (exitError) {
        reject(exitError)
        return
      }
      if (exitResult) {
        if (exitResult.code === 0) resolve(`${stdout}\n${stderr}`)
        else reject(new Error(`opencode run --attach exited with ${exitResult.signal ?? exitResult.code}\n${stdout}\n${stderr}`))
        return
      }
      const timer = setTimeout(() => {
        child.kill("SIGTERM")
        reject(new Error(`opencode run --attach timed out for ${providerSessionId}\n${stdout}\n${stderr}`))
      }, timeoutMs)
      child.once("error", (error) => {
        clearTimeout(timer)
        reject(error)
      })
      child.once("exit", (code, signal) => {
        clearTimeout(timer)
        if (code === 0) resolve(`${stdout}\n${stderr}`)
        else reject(new Error(`opencode run --attach exited with ${signal ?? code}\n${stdout}\n${stderr}`))
      })
    }),
  }
}

export async function codexRpc(proxyUrl, messages, timeoutMs = 30_000) {
  return await new Promise((resolve, reject) => {
    const ws = new WebSocket(proxyUrl)
    const responses = []
    const timer = setTimeout(() => {
      ws.close()
      reject(new Error(`codex rpc timed out; responses=${JSON.stringify(responses)}`))
    }, timeoutMs)
    ws.once("open", () => {
      for (const message of messages) ws.send(JSON.stringify(message))
    })
    ws.on("message", (raw) => {
      let message = null
      try {
        message = JSON.parse(raw.toString())
      } catch {
        return
      }
      responses.push(message)
      const wanted = new Set(messages.filter((entry) => entry.id !== undefined).map((entry) => entry.id))
      const received = new Set(responses.filter((entry) => entry.id !== undefined).map((entry) => entry.id))
      if ([...wanted].every((id) => received.has(id))) {
        clearTimeout(timer)
        ws.close()
        resolve(responses)
      }
    })
    ws.once("error", (error) => {
      clearTimeout(timer)
      reject(error)
    })
  })
}

export async function runNativeCodexPrompt(proxyUrl, threadId, prompt, extraInput = []) {
  const responses = await codexRpc(proxyUrl, [
    {
      id: 2,
      method: "turn/start",
      params: {
        threadId,
        input: [{ type: "text", text: prompt, text_elements: [] }, ...extraInput],
      },
    },
  ])
  const turnResponse = responses.find((response) => response.id === 2)
  if (!turnResponse || turnResponse.error) {
    throw new Error(`codex native turn failed: ${JSON.stringify(turnResponse)}`)
  }
}

export async function sendClaudeRenderedPromptViaKernelInput(client, sessionId, attachmentId, providerRunId, prompt) {
  await client.send(sendTerminalInputRequest(sessionId, attachmentId, prompt, providerRunId))
  await sleep(250)
  await client.send(sendTerminalInputRequest(sessionId, attachmentId, "\r", providerRunId))
}
