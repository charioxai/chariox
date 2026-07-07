#!/usr/bin/env node
import { execFileSync } from "node:child_process"
import { existsSync, readFileSync } from "node:fs"

const maxLines = Number.parseInt(process.env.ARROBA_LINE_CAP_MAX ?? "1000", 10)
const allowlistPath = new URL("./line-cap-allowlist.txt", import.meta.url)

const excludedPathPatterns = [
  /^pnpm-lock\.yaml$/,
  /^package-lock\.json$/,
  /^yarn\.lock$/,
  /^Cargo\.lock$/,
  /^core$/,
  /^\.artifacts\//,
  /^tmp\//,
  /^host-worktree\//,
  /^codex-main\//,
  /^happy-main\//,
  /^opencode-dev\//,
  /^paseo\//,
  /\.(mjsfrag|tsfrag|png|jpg|jpeg|gif|webp|ico|pdf|zip|gz|tar|tgz|wav|mp3|mp4|mov)$/i,
]

const trackedFiles = execFileSync("git", ["ls-files", "-z"], { encoding: "utf8" })
  .split("\0")
  .filter(Boolean)

const allowlist = existsSync(allowlistPath)
  ? new Set(
      readFileSync(allowlistPath, "utf8")
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter((line) => line && !line.startsWith("#")),
    )
  : new Set()

const isExcluded = (path) => excludedPathPatterns.some((pattern) => pattern.test(path))

const lineCount = (path) => {
  const content = readFileSync(path, "utf8")
  if (content.length === 0) return 0
  return content.endsWith("\n") ? content.split("\n").length - 1 : content.split("\n").length
}

const oversized = []
const unreadable = []

for (const path of trackedFiles) {
  if (isExcluded(path)) continue
  try {
    const lines = lineCount(path)
    if (lines > maxLines) oversized.push({ path, lines })
  } catch (error) {
    unreadable.push({ path, error })
  }
}

const oversizedPaths = new Set(oversized.map((entry) => entry.path))
const newOffenders = oversized.filter((entry) => !allowlist.has(entry.path))
const staleAllowlist = [...allowlist].filter((path) => !oversizedPaths.has(path)).sort()

if (unreadable.length > 0) {
  console.error("line-cap: could not read tracked text files")
  for (const entry of unreadable) {
    console.error(`  ${entry.path}: ${entry.error.message}`)
  }
  process.exit(1)
}

if (newOffenders.length > 0) {
  console.error(`line-cap: ${newOffenders.length} hand-authored file(s) exceed ${maxLines} lines`)
  for (const entry of newOffenders.sort((a, b) => b.lines - a.lines || a.path.localeCompare(b.path))) {
    console.error(`  ${entry.lines.toString().padStart(5, " ")} ${entry.path}`)
  }
  console.error(`Add only current migration targets to ${allowlistPath.pathname} or split the files.`)
  process.exit(1)
}

if (staleAllowlist.length > 0) {
  console.error("line-cap: allowlist contains entries that are no longer oversized")
  for (const path of staleAllowlist) console.error(`  ${path}`)
  console.error("Remove stale entries so the allowlist shrinks during the migration.")
  process.exit(1)
}

console.log(
  `line-cap: ${oversized.length} allowlisted file(s) over ${maxLines} lines; no new offenders`,
)
