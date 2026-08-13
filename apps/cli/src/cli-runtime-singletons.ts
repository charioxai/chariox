import process from "node:process"

import { addDefaultParsers } from "@opentui/core"

import {
  createCliProcessLoggerRegistry,
  formatCliError,
} from "./cli-process-logging.js"
import { createTranscriptParserRegistration } from "./transcript-parser-registration.js"
import parserConfig from "./parsers-config.js"

export const DEBUG_LOGS_ENABLED = (process.env.CHARIOX_LOG_LEVEL ?? "").toLowerCase() === "debug"
export const OPEN_CONSOLE_ON_ERROR = process.env.CHARIOX_OPEN_CONSOLE_ON_ERROR === "1"

export const processLoggers = createCliProcessLoggerRegistry()
export const getLogger = processLoggers.getLogger
export const formatError = formatCliError

export const transcriptParserRegistration = createTranscriptParserRegistration({
  parsers: parserConfig.parsers,
  addDefaultParsers: (parsers) => {
    addDefaultParsers([...parsers])
  },
})
