// MP-08 / MP-10 / MP-11: match the native launcher's installed-harness ancestry.
import { execFileSync } from "node:child_process"
import { realpathSync, readlinkSync, readdirSync, accessSync, constants } from "node:fs"
import { dirname, join, basename } from "node:path"
import { homedir } from "node:os"

function canonical(path: string): string | null {
  try { return realpathSync(path) } catch { return null }
}

function installed(name: string): string | null {
  const override = process.env[`CHARIOX_${name.toUpperCase()}_BIN`]
  const candidates = override ? [override] : [
    ...(process.env.PATH ?? "").split(":").filter(Boolean).map(dir => join(dir, name)),
    ...(name === "opencode" ? [join(homedir(), ".opencode/bin/opencode")] : []),
  ]
  for (const candidate of candidates) {
    try { accessSync(candidate, constants.X_OK) } catch { continue }
    const path = canonical(candidate)
    if (path) return path
  }
  return null
}

function harnessPaths(): Set<string> {
  const paths = new Set<string>()
  for (const name of ["codex", "claude", "opencode"]) {
    const configured = installed(name)
    if (!configured) continue
    paths.add(configured)
    if (name === "claude" && /^\d+\.\d+\.\d+$/.test(basename(configured)) && basename(dirname(configured)) === "versions"
      && basename(dirname(dirname(configured))) === "claude") {
      for (const version of readdirSync(dirname(configured))) {
        if (/^\d+\.\d+\.\d+$/.test(version)) {
          const candidate = join(dirname(configured), version)
          if (canonical(candidate) === candidate) paths.add(candidate)
        }
      }
    }
    if (name === "codex" && basename(configured) === "codex.js") {
      const platforms: Record<string, readonly [string, string]> = {
        "linux:x64": ["linux-x64", "x86_64-unknown-linux-musl"],
        "linux:arm64": ["linux-arm64", "aarch64-unknown-linux-musl"],
        "darwin:x64": ["darwin-x64", "x86_64-apple-darwin"],
        "darwin:arm64": ["darwin-arm64", "aarch64-apple-darwin"],
      }
      const platform = platforms[`${process.platform}:${process.arch}`]
      if (!platform) continue
      const root = dirname(dirname(configured))
      const native = `vendor/${platform[1]}/bin/codex`
      for (const candidate of [join(root, native), join(root, "node_modules/@openai", `codex-${platform[0]}`, native), join(dirname(root), `codex-${platform[0]}`, native)]) {
        const path = canonical(candidate)
        if (path) paths.add(path)
      }
    }
  }
  return paths
}

export function defaultAccessHolder(): number {
  const override = process.env.CHARIOX_CLI_ACCESS_HOLDER_PID
  if (override !== undefined) return Number(override)
  const installedHarnesses = harnessPaths()
  let pid = process.pid
  let fallback: number | undefined
  const visited = new Set<number>()
  for (let depth = 0; pid > 1 && depth < 128 && !visited.has(pid); depth++) {
    visited.add(pid)
    const info = execFileSync("/bin/ps", ["-o", "ppid=,uid=", "-p", String(pid)], { encoding: "utf8" }).trim().split(/\s+/).map(Number)
    const [parent, uid] = info
    if (depth === 2) fallback = pid
    if (depth > 0 && uid === process.getuid?.()) {
      const executable = process.platform === "linux"
        ? readlinkSync(`/proc/${pid}/exe`)
        : execFileSync("/bin/ps", ["-o", "comm=", "-p", String(pid)], { encoding: "utf8" }).trim()
      if (installedHarnesses.has(executable)) return pid
    }
    if (typeof parent !== "number" || !Number.isSafeInteger(parent) || parent <= 1) break
    pid = parent
  }
  if (fallback !== undefined) return fallback
  throw new Error("Cannot determine holder; provide --holder-pid")
}
