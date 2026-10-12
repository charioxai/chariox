// MP-08 / MP-10 / MP-11: screen is a client attachment to the kernel-owned TUI.
import { execFile } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
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
  const bannerFile = path.join(root, "banner.txt")
  let started = false
  try {
    await execFileAsync("screen", ["-dmS", name, ...command], {
      cwd: worktree,
      env: { ...process.env, CHARIOX_CLAUDE_NATIVE_BANNER_FILE: bannerFile },
    })
    started = true
    process.stderr.write(`Claude native TUI client: screen: ${name}\n`)
    let forwarded = false
    do {
      if (!forwarded) {
        const banner = await readFile(bannerFile, "utf8").catch(() => "")
        if (banner) { process.stderr.write(banner); forwarded = true }
      }
      if (!await screenExists(name)) break
      await sleep(500)
    } while (true)
    if (!forwarded) {
      const banner = await readFile(bannerFile, "utf8").catch(() => "")
      if (banner) { process.stderr.write(banner); forwarded = true }
    }
    if (!forwarded) throw new Error("Claude native TUI client exited before its kernel runtime banner was ready")
  } finally {
    if (started) await execFileAsync("screen", ["-S", name, "-X", "quit"]).catch(() => {})
    await rm(root, { recursive: true, force: true })
  }
}

async function screenExists(name: string): Promise<boolean> {
  // Stock macOS screen returns 1 even while listing a live session.
  const { stdout } = await execFileAsync("screen", ["-ls"]).catch((error: { stdout?: string }) => ({ stdout: error.stdout ?? "" }))
  return stdout.split("\n").some(line => line.trim().split(/\s+/)[0]?.endsWith(`.${name}`))
}
