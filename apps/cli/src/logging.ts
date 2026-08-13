import { appendFileSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync } from "node:fs"
import path from "node:path"
import process from "node:process"
import { homedir } from "node:os"

const DEFAULT_LOG_RETENTION_DAYS = 7
const DEFAULT_LOG_ROOT_MAX_BYTES = 200 * 1024 * 1024

export type LogLevel = "debug" | "info" | "warn" | "error" | "off"
export type LogFields = Record<string, unknown>

export type LogRecord = {
  timestamp_ms: number
  level: LogLevel
  process_kind: string
  pid: number
  component: string
  message: string
  log_path?: string
  session_id?: string
  provider_run_id?: string
  attachment_id?: string
  client_id?: string
  request_id?: string
  trace_id?: string
  [key: string]: unknown
}

type LogSink = {
  processKind: string
  pid: number
  logPath: string
  level: LogLevel
}

const LEVEL_ORDER: Record<LogLevel, number> = {
  debug: 10,
  info: 20,
  warn: 30,
  error: 40,
  off: 100,
}

export class CharioxLogger {
  constructor(
    private readonly sink: LogSink,
    private readonly component: string,
    private readonly context: LogFields = {},
  ) {}

  child(component: string, context: LogFields = {}) {
    return new CharioxLogger(this.sink, component, { ...this.context, ...context })
  }

  debug(message: string, fields: LogFields = {}) {
    this.write("debug", message, fields)
  }

  info(message: string, fields: LogFields = {}) {
    this.write("info", message, fields)
  }

  warn(message: string, fields: LogFields = {}) {
    this.write("warn", message, fields)
  }

  error(message: string, fields: LogFields = {}) {
    this.write("error", message, fields)
  }

  private write(level: LogLevel, message: string, fields: LogFields) {
    if (LEVEL_ORDER[level] < LEVEL_ORDER[this.sink.level]) {
      return
    }

    const record: LogRecord = {
      timestamp_ms: Date.now(),
      level,
      process_kind: this.sink.processKind,
      pid: this.sink.pid,
      component: this.component,
      message,
      log_path: this.sink.logPath,
      ...this.context,
      ...fields,
    }

    try {
      appendFileSync(this.sink.logPath, `${JSON.stringify(record)}\n`, "utf8")
    } catch {}
  }
}

export function createProcessLogger(processKind: string, component = "logging"): CharioxLogger {
  const logDir = defaultLogDir()
  mkdirSync(logDir, { recursive: true })
  cleanupLogDir(logDir)

  const level = configuredLogLevel()
  const logPath = path.join(logDir, `${Date.now()}-${processKind}-${process.pid}.ndjson`)
  const sink: LogSink = {
    processKind,
    pid: process.pid,
    logPath,
    level,
  }
  const logger = new CharioxLogger(sink, component)
  if (level !== "off") {
    logger.info("initialized process logger", {
      log_root: logDir,
      log_path: logPath,
    })
  }
  return logger
}

export function defaultLogDir() {
  if (process.env.CHARIOX_LOG_DIR) {
    return process.env.CHARIOX_LOG_DIR
  }

  const workspaceLogDir = path.join(process.cwd(), ".chariox", "logs")
  if (workspaceLogDir) {
    return workspaceLogDir
  }

  if (process.env.XDG_STATE_HOME) {
    return path.join(process.env.XDG_STATE_HOME, "chariox", "logs")
  }

  if (homedir()) {
    return path.join(homedir(), ".local", "state", "chariox", "logs")
  }

  return path.join(process.cwd(), ".chariox", "logs")
}

export function readAllLogRecords(logDir = defaultLogDir()): LogRecord[] {
  const records: LogRecord[] = []
  for (const filePath of listLogFiles(logDir)) {
    const contents = safeReadText(filePath)
    if (!contents) {
      continue
    }
    for (const line of contents.split("\n")) {
      if (!line.trim()) {
        continue
      }
      try {
        records.push(JSON.parse(line) as LogRecord)
      } catch {}
    }
  }
  return records.sort((left, right) => left.timestamp_ms - right.timestamp_ms)
}

export function listLogFiles(logDir = defaultLogDir()) {
  try {
    return readdirSync(logDir)
      .filter((entry) => entry.endsWith(".ndjson"))
      .map((entry) => path.join(logDir, entry))
      .sort()
  } catch {
    return []
  }
}

export function formatLogRecord(record: LogRecord) {
  const timestamp = new Date(record.timestamp_ms).toISOString()
  const scope = [
    record.process_kind,
    record.component,
    record.session_id ? `session=${record.session_id}` : null,
    record.provider_run_id ? `run=${record.provider_run_id}` : null,
    record.client_id ? `client=${record.client_id}` : null,
  ]
    .filter(Boolean)
    .join(" ")

  return `[${timestamp}] ${record.level.toUpperCase()} ${scope} ${record.message}`
}

export function readLogTail(logDir: string, offsets: Map<string, number>) {
  const records: LogRecord[] = []

  for (const filePath of listLogFiles(logDir)) {
    const previousOffset = offsets.get(filePath) ?? 0
    const contents = safeReadBytes(filePath)
    if (contents == null) {
      continue
    }
    const currentSize = contents.length
    const nextOffset = previousOffset > currentSize ? 0 : previousOffset
    const nextContents = contents.subarray(nextOffset).toString("utf8")
    offsets.set(filePath, currentSize)

    for (const line of nextContents.split("\n")) {
      if (!line.trim()) {
        continue
      }
      try {
        records.push(JSON.parse(line) as LogRecord)
      } catch {}
    }
  }

  return records.sort((left, right) => left.timestamp_ms - right.timestamp_ms)
}

export function seedLogOffsets(logDir = defaultLogDir()) {
  const offsets = new Map<string, number>()
  for (const filePath of listLogFiles(logDir)) {
    try {
      offsets.set(filePath, statSync(filePath).size)
    } catch {}
  }
  return offsets
}

function configuredLogLevel(): LogLevel {
  const value = (process.env.CHARIOX_LOG_LEVEL ?? "info").toLowerCase()
  if (value === "debug" || value === "warn" || value === "error" || value === "off") {
    return value
  }
  return "info"
}

function cleanupLogDir(logDir: string) {
  try {
    const now = Date.now()
    const retentionMs = DEFAULT_LOG_RETENTION_DAYS * 24 * 60 * 60 * 1000
    const entries = listLogFiles(logDir).map((filePath) => {
      const stats = statSync(filePath)
      return {
        filePath,
        size: stats.size,
        mtimeMs: stats.mtimeMs,
      }
    })

    for (const entry of entries) {
      if (now - entry.mtimeMs > retentionMs) {
        rmSync(entry.filePath, { force: true })
      }
    }

    const freshEntries = listLogFiles(logDir).map((filePath) => {
      const stats = statSync(filePath)
      return {
        filePath,
        size: stats.size,
        mtimeMs: stats.mtimeMs,
      }
    })

    let total = freshEntries.reduce((sum, entry) => sum + entry.size, 0)
    if (total <= DEFAULT_LOG_ROOT_MAX_BYTES) {
      return
    }

    freshEntries.sort((left, right) => left.mtimeMs - right.mtimeMs)
    for (const entry of freshEntries) {
      if (total <= DEFAULT_LOG_ROOT_MAX_BYTES) {
        break
      }
      rmSync(entry.filePath, { force: true })
      total -= entry.size
    }
  } catch {}
}

function safeReadText(filePath: string) {
  try {
    return readFileSync(filePath, "utf8")
  } catch {
    return null
  }
}

function safeReadBytes(filePath: string) {
  try {
    return readFileSync(filePath)
  } catch {
    return null
  }
}
