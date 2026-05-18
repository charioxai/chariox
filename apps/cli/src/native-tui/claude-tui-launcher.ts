import { execFile, spawn } from "node:child_process"
import { readFile } from "node:fs/promises"
import path from "node:path"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"
import { promisify } from "node:util"

import { shellQuote } from "./launch-environment.js"

const execFileAsync = promisify(execFile)

export type ClaudeTuiController = {
  label: string
  submitPrompt: (prompt: string) => Promise<void>
  waitForExit: () => Promise<void>
  stop: () => Promise<void>
}

export async function startClaudeScreen(name: string, logDir: string, options: {
  worktree: string
  settingsPath: string
  model: string
  effort: string
  permissions: "required" | "yolo"
  env: NodeJS.ProcessEnv
}): Promise<ClaudeTuiController> {
  const claudeArgs = claudeCommandArgs(options)
  await execFileAsync("screen", [
    "-dmS",
    name,
    "-L",
    "bash",
    "-lc",
    `cd ${shellQuote(options.worktree)} && exec ${claudeArgs.map(shellQuote).join(" ")}`,
  ], {
    cwd: logDir,
    env: options.env,
  })
  return {
    label: `screen:${name}`,
    submitPrompt: async (prompt) => {
      await waitForScreenReady(logDir, name)
      await submitScreenPrompt(name, prompt)
    },
    waitForExit: async () => {
      while (await screenExists(name)) await sleep(500)
    },
    stop: () => screenQuit(name),
  }
}

export async function startClaudeAttachedPty(options: {
  worktree: string
  settingsPath: string
  model: string
  effort: string
  permissions: "required" | "yolo"
  env: NodeJS.ProcessEnv
}): Promise<ClaudeTuiController> {
  const command = `cd ${shellQuote(options.worktree)} && exec ${claudeCommandArgs(options).map(shellQuote).join(" ")}`
  const child = spawn("script", scriptArgs(command), {
    cwd: options.worktree,
    env: options.env,
    stdio: ["pipe", "pipe", "pipe"],
  })
  child.stdout?.on("data", (chunk) => process.stdout.write(chunk))
  child.stderr?.on("data", (chunk) => process.stderr.write(chunk))

  const stdin = child.stdin
  if (!stdin) {
    child.kill("SIGTERM")
    throw new Error("failed to start attached Claude PTY: script stdin was unavailable")
  }
  const ready = sleep(1_500)

  const forwardInput = (chunk: Buffer) => {
    if (!stdin.destroyed) stdin.write(chunk)
  }
  const wasRaw = Boolean(process.stdin.isTTY && process.stdin.isRaw)
  if (process.stdin.isTTY) process.stdin.setRawMode?.(true)
  process.stdin.resume()
  process.stdin.on("data", forwardInput)

  let stopped = false
  const waitForExit = new Promise<void>((resolve, reject) => {
    child.once("error", (error) => reject(new Error(`failed to start attached Claude PTY via script: ${error.message}`)))
    child.once("exit", () => resolve())
  }).finally(() => {
    process.stdin.off("data", forwardInput)
    if (process.stdin.isTTY) process.stdin.setRawMode?.(wasRaw)
  })

  return {
    label: "attached-pty",
    submitPrompt: async (prompt) => {
      await ready
      if (stdin.destroyed) throw new Error("attached Claude PTY is closed")
      stdin.write(prompt)
      await sleep(250)
      stdin.write("\r")
    },
    waitForExit: () => waitForExit,
    stop: async () => {
      if (stopped) return
      stopped = true
      if (child.exitCode == null && child.signalCode == null) {
        child.kill("SIGTERM")
        await Promise.race([waitForExit, sleep(2_000)]).catch(() => {})
        if (child.exitCode == null && child.signalCode == null) child.kill("SIGKILL")
      }
    },
  }
}

function claudeCommandArgs(options: {
  settingsPath: string
  model: string
  effort: string
  permissions: "required" | "yolo"
}): string[] {
  return [
    "claude",
    "--settings",
    options.settingsPath,
    "--permission-mode",
    options.permissions === "yolo" ? "bypassPermissions" : "default",
    "--model",
    options.model,
    "--effort",
    options.effort,
  ]
}

function scriptArgs(command: string): string[] {
  if (process.platform === "linux") return ["-q", "-c", command, "/dev/null"]
  return ["-q", "/dev/null", "bash", "-lc", command]
}

async function waitForScreenReady(logDir: string, name: string) {
  const logPath = path.join(logDir, "screenlog.0")
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    if (!(await screenExists(name))) throw new Error(`Claude TUI screen exited before it became ready: ${name}`)
    const log = await readFile(logPath, "utf8").catch(() => "")
    if (log.includes("Claude") && log.includes("Code")) return
    await sleep(250)
  }
  throw new Error(`timed out waiting for Claude TUI screen to become ready: ${name}`)
}

async function screenStuff(name: string, text: string) {
  await execFileAsync("screen", ["-S", name, "-p", "0", "-X", "stuff", text])
}

async function submitScreenPrompt(name: string, prompt: string) {
  await screenStuff(name, prompt)
  await sleep(250)
  await screenStuff(name, "\r")
}

async function screenQuit(name: string) {
  await execFileAsync("screen", ["-S", name, "-p", "0", "-X", "quit"]).catch(() => {})
}

async function screenExists(name: string): Promise<boolean> {
  try {
    const { stdout } = await execFileAsync("screen", ["-ls"])
    return stdout.includes(`.${name}`)
  } catch (error) {
    const output = typeof error === "object" && error && "stdout" in error
      ? String((error as { stdout?: unknown }).stdout ?? "")
      : ""
    return output.includes(`.${name}`)
  }
}
