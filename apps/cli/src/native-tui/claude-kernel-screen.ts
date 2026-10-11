// MP-08 / MP-10 / MP-11: screen is a client attachment to the kernel-owned TUI.
import { execFile } from "node:child_process"
import { mkdtemp, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"
import { promisify } from "node:util"
import { releaseVersion } from "../release-build.js"

const execFileAsync = promisify(execFile)

export async function runClaudeKernelScreen(args: string[], worktree: string): Promise<void> {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-claude-native-screen-"))
  const name = `chariox-claude-${process.pid}-${Date.now()}`
  const command = releaseVersion === undefined
    ? [process.execPath, process.argv[1]!, "claude", ...args]
    : [process.execPath, "claude", ...args]
  let started = false
  try {
    await execFileAsync("screen", ["-dmS", name, "-L", "-Logfile", path.join(root, "screen.log"), ...command], { cwd: worktree })
    started = true
    process.stderr.write(`Claude native TUI client: screen:${name}\n`)
    while (await screenExists(name)) await sleep(500)
  } finally {
    if (started) await execFileAsync("screen", ["-S", name, "-X", "quit"]).catch(() => {})
    await rm(root, { recursive: true, force: true })
  }
}

async function screenExists(name: string): Promise<boolean> {
  try {
    const { stdout } = await execFileAsync("screen", ["-ls"])
    return stdout.split("\n").some(line => line.trim().split(/\s+/)[0]?.endsWith(`.${name}`))
  } catch { return false }
}
