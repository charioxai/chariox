#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process"
import { pathToFileURL } from "node:url"
import { createInterface } from "node:readline"

export function parseChangedRanges(patch) {
  const ranges = new Map()
  let file = null
  for (const line of patch.split("\n")) {
    if (line.startsWith("+++ b/")) {
      file = line.slice(6)
      continue
    }
    if (!file || !line.startsWith("@@")) continue
    const match = line.match(/\+(\d+)(?:,(\d+))?/)
    if (!match) continue
    const start = Number(match[1])
    const count = Number(match[2] ?? "1")
    if (count === 0) continue
    const fileRanges = ranges.get(file) ?? []
    fileRanges.push([start, start + count - 1])
    ranges.set(file, fileRanges)
  }
  return ranges
}

export function lineIsChanged(ranges, file, line) {
  return (ranges.get(file) ?? []).some(([start, end]) => line >= start && line <= end)
}

async function main() {
  const explicitBase = process.argv[2]?.trim()
  const base = explicitBase || (process.env.GITHUB_BASE_REF ? `origin/${process.env.GITHUB_BASE_REF}` : "HEAD^")

  const diff = spawnSync(
    "git",
    ["diff", "--unified=0", "--no-color", `${base}...HEAD`, "--", "*.rs"],
    { encoding: "utf8" },
  )
  if (diff.status !== 0) {
    process.stderr.write(diff.stderr)
    process.exit(diff.status ?? 1)
  }

  const changedRanges = parseChangedRanges(diff.stdout)
  const child = spawn(
    "cargo",
    [
      "clippy",
      "--workspace",
      "--all-targets",
      "--all-features",
      "--message-format=json",
    ],
    { stdio: ["ignore", "pipe", "inherit"] },
  )

  const regressions = new Map()
  let compilerError = false
  const lines = createInterface({ input: child.stdout })
  for await (const line of lines) {
    let event
    try {
      event = JSON.parse(line)
    } catch {
      continue
    }
    if (event.reason !== "compiler-message") continue
    const message = event.message
    if (message?.level === "error") compilerError = true
    if (message?.level !== "warning") continue
    for (const span of message.spans ?? []) {
      if (!span.is_primary || !lineIsChanged(changedRanges, span.file_name, span.line_start)) continue
      const code = message.code?.code ?? "warning"
      const key = `${span.file_name}:${span.line_start}:${span.column_start}:${code}:${message.message}`
      regressions.set(key, {
        code,
        column: span.column_start,
        file: span.file_name,
        line: span.line_start,
        message: message.message,
      })
    }
  }

  const exitCode = await new Promise((resolve) => child.on("close", resolve))
  if (exitCode !== 0 || compilerError) {
    console.error(`Clippy could not analyze the workspace (exit ${exitCode ?? "unknown"}).`)
    process.exit(exitCode || 1)
  }

  if (regressions.size > 0) {
    console.error(`Clippy found ${regressions.size} warning(s) on changed Rust lines:`)
    for (const diagnostic of regressions.values()) {
      console.error(
        `${diagnostic.file}:${diagnostic.line}:${diagnostic.column}: ${diagnostic.code}: ${diagnostic.message}`,
      )
    }
    process.exit(1)
  }

  console.log(
    `Clippy analyzed the full workspace; no warnings intersect Rust lines changed from ${base}.`,
  )
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main()
}
