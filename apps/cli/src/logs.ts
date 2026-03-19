import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

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
}

export async function runLogViewer(args: string[]) {
  const options = parseLogViewerArgs(args)
  const logDir = defaultLogDir()
  const matching = readAllLogRecords(logDir)
    .filter((record) => recordMatches(record, options))
    .slice(-options.limit)

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

function parseLogViewerArgs(args: string[]): LogViewerOptions {
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
      case "--help":
      case "-h":
        printLogUsage()
        process.exit(0)
      default:
        throw new Error(`unknown logs argument ${arg}`)
    }
  }

  return options
}

function recordMatches(record: LogRecord, options: LogViewerOptions) {
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
    "usage: arroba-cli logs [--follow] [--process-kind KIND] [--component NAME] [--session ID] [--provider-run ID] [--client-id ID] [--level LEVEL] [--limit N]\n",
  )
}
