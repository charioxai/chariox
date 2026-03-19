import { appendFileSync, mkdirSync, openSync, readdirSync, rmSync, statSync } from "node:fs"
import { homedir } from "node:os"
import path from "node:path"
import process from "node:process"

const DEFAULT_LOG_RETENTION_DAYS = 7
const DEFAULT_LOG_ROOT_MAX_BYTES = 200 * 1024 * 1024

type LogLevel = "debug" | "info" | "warn" | "error" | "off"
type LogFields = Record<string, unknown>

const LEVEL_ORDER: Record<LogLevel, number> = {
  debug: 10,
  info: 20,
  warn: 30,
  error: 40,
  off: 100,
}

type LogSink = {
  processKind: string
  pid: number
  logPath: string
  level: LogLevel
}

export class ArrobaLogger {
  constructor(
    private readonly sink: LogSink,
    private readonly component: string,
    private readonly context: LogFields = {},
  ) {}

  child(component: string, context: LogFields = {}) {
    return new ArrobaLogger(this.sink, component, { ...this.context, ...context })
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

    try {
      appendFileSync(
        this.sink.logPath,
        `${JSON.stringify({
          timestamp_ms: Date.now(),
          level,
          process_kind: this.sink.processKind,
          pid: this.sink.pid,
          component: this.component,
          message,
          log_path: this.sink.logPath,
          ...this.context,
          ...fields,
        })}\n`,
        "utf8",
      )
    } catch {}
  }
}

export function createProcessLogger(processKind: string, component = "logging") {
  const logDir = defaultLogDir()
  mkdirSync(logDir, { recursive: true })
  cleanupLogDir(logDir)

  const level = configuredLogLevel()
  const logPath = path.join(logDir, `${Date.now()}-${processKind}-${process.pid}.ndjson`)
  if (level !== "off") {
    try {
      openSync(logPath, "a")
    } catch {}
  }

  const logger = new ArrobaLogger(
    {
      processKind,
      pid: process.pid,
      logPath,
      level,
    },
    component,
  )

  if (level !== "off") {
    logger.info("initialized process logger", {
      log_root: logDir,
      log_path: logPath,
    })
  }

  return logger
}

function configuredLogLevel(): LogLevel {
  const value = (process.env.ARROBA_LOG_LEVEL ?? "info").toLowerCase()
  if (value === "debug" || value === "warn" || value === "error" || value === "off") {
    return value
  }
  return "info"
}

function defaultLogDir() {
  if (process.env.ARROBA_LOG_DIR) {
    return process.env.ARROBA_LOG_DIR
  }

  if (process.env.XDG_STATE_HOME) {
    return path.join(process.env.XDG_STATE_HOME, "arroba", "logs")
  }

  if (homedir()) {
    return path.join(homedir(), ".local", "state", "arroba", "logs")
  }

  return path.join(process.cwd(), ".arroba", "logs")
}

function cleanupLogDir(logDir: string) {
  try {
    const now = Date.now()
    const retentionMs = DEFAULT_LOG_RETENTION_DAYS * 24 * 60 * 60 * 1000
    const entries = readdirSync(logDir)
      .filter((entry) => entry.endsWith(".ndjson"))
      .map((entry) => {
        const filePath = path.join(logDir, entry)
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

    const freshEntries = readdirSync(logDir)
      .filter((entry) => entry.endsWith(".ndjson"))
      .map((entry) => {
        const filePath = path.join(logDir, entry)
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
