import {
  createProcessLogger,
  type CharioxLogger,
  type LogFields,
} from "./logging.js"
import { describeCliError } from "./runtime.js"

export type CreateCliProcessLogger = (processKind: string, component?: string) => CharioxLogger

export function createCliProcessLoggerRegistry(options: {
  createLogger?: CreateCliProcessLogger
} = {}) {
  const createLogger = options.createLogger ?? createProcessLogger
  let processLogger: CharioxLogger | null = null

  return {
    initialize(processKind: string) {
      processLogger = createLogger(processKind)
      return processLogger
    },

    getLogger(component: string, fields: LogFields = {}) {
      return processLogger?.child(component, fields) ?? null
    },
  }
}

export function formatCliError(error: unknown): string {
  return describeCliError(error)
}
