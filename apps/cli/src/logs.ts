import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"
import { mkdirSync, writeFileSync } from "node:fs"
import path from "node:path"

import { defaultLogDir, formatLogRecord, readAllLogRecords, readLogTail, seedLogOffsets, type LogLevel, type LogRecord } from "./logging.js"

type LogViewerOptions = {
  follow: boolean
  component?: string
  processKind?: string
  sessionId?: string
  providerRunId?: string
  clientId?: string
  level?: LogLevel
  limit: number
  bundleDir?: string
}

export async function runLogViewer(args: string[]) {
  const options = parseLogViewerArgs(args)
  const logDir = defaultLogDir()
  const matching = matchingLogRecords(logDir, options)

  if (options.bundleDir) {
    const bundle = writeLogBundle(logDir, options.bundleDir, options, matching)
    process.stdout.write(`wrote Chariox log bundle: ${bundle.bundleDir}\n`)
    process.stdout.write(`records: ${bundle.recordCount}\n`)
    return
  }

  for (const record of matching) {
    process.stdout.write(`${formatLogRecord(record)}\n`)
  }

  if (!options.follow) {
    return
  }

  const offsets = seedLogOffsets(logDir)

  while (true) {
    const tail = readLogTail(logDir, offsets)
      .filter((record) => recordMatches(record, options))
    for (const record of tail) {
      process.stdout.write(`${formatLogRecord(record)}\n`)
    }
    await sleep(500)
  }
}

export function parseLogViewerArgs(args: string[]): LogViewerOptions {
  const options: LogViewerOptions = {
    follow: false,
    limit: 200,
  }

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    const next = () => {
      const value = args[index + 1]
      if (!value) {
        throw new Error(`missing value for ${arg}`)
      }
      index += 1
      return value
    }

    switch (arg) {
      case "--follow":
        options.follow = true
        break
      case "--component":
        options.component = next()
        break
      case "--process-kind":
        options.processKind = next()
        break
      case "--session":
        options.sessionId = next()
        break
      case "--provider-run":
        options.providerRunId = next()
        break
      case "--client-id":
        options.clientId = next()
        break
      case "--level":
        options.level = next() as LogLevel
        break
      case "--limit":
        options.limit = Number.parseInt(next(), 10) || 200
        break
      case "--bundle":
        options.bundleDir = next()
        break
      case "--help":
      case "-h":
        printLogUsage()
        process.exit(0)
      default:
        throw new Error(`unknown logs argument ${arg}`)
    }
  }

  if (options.follow && options.bundleDir) {
    throw new Error("logs --bundle cannot be combined with --follow")
  }

  return options
}

export function matchingLogRecords(logDir: string, options: Pick<LogViewerOptions, "component" | "processKind" | "sessionId" | "providerRunId" | "clientId" | "level" | "limit">) {
  return readAllLogRecords(logDir)
    .filter((record) => recordMatches(record, options))
    .slice(-options.limit)
}

export function writeLogBundle(
  logDir: string,
  bundleDir: string,
  options: Pick<LogViewerOptions, "component" | "processKind" | "sessionId" | "providerRunId" | "clientId" | "level" | "limit">,
  records: readonly LogRecord[],
) {
  const resolvedBundleDir = path.resolve(bundleDir)
  mkdirSync(resolvedBundleDir, { recursive: true })
  const manifest = {
    schema: "chariox.log_bundle.v1",
    created_at_ms: Date.now(),
    log_root: logDir,
    filters: {
      process_kind: options.processKind ?? null,
      component: options.component ?? null,
      session_id: options.sessionId ?? null,
      provider_run_id: options.providerRunId ?? null,
      client_id: options.clientId ?? null,
      level: options.level ?? null,
      limit: options.limit,
    },
    record_count: records.length,
    files: ["logs.ndjson"],
  }
  writeFileSync(path.join(resolvedBundleDir, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`, "utf8")
  writeFileSync(
    path.join(resolvedBundleDir, "logs.ndjson"),
    records.map((record) => JSON.stringify(record)).join("\n") + (records.length ? "\n" : ""),
    "utf8",
  )
  return {
    bundleDir: resolvedBundleDir,
    recordCount: records.length,
  }
}

function recordMatches(record: LogRecord, options: Pick<LogViewerOptions, "component" | "processKind" | "sessionId" | "providerRunId" | "clientId" | "level">) {
  if (options.component && record.component !== options.component) {
    return false
  }
  if (options.processKind && record.process_kind !== options.processKind) {
    return false
  }
  if (options.sessionId && record.session_id !== options.sessionId) {
    return false
  }
  if (options.providerRunId && record.provider_run_id !== options.providerRunId) {
    return false
  }
  if (options.clientId && record.client_id !== options.clientId) {
    return false
  }
  if (options.level && record.level !== options.level) {
    return false
  }
  return true
}

export function printLogUsage() {
  process.stdout.write(
    "usage: chariox-cli logs [--follow] [--process-kind KIND] [--component NAME] [--session ID] [--provider-run ID] [--client-id ID] [--level LEVEL] [--limit N] [--bundle DIR]\n",
  )
}
