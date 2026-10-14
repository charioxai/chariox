import { type IncomingMessage, type ServerResponse } from "node:http"
import { Readable, Transform } from "node:stream"
import { StringDecoder } from "node:string_decoder"

import { redactHiddenInstructions, redactHiddenInstructionsFromJson } from "./hidden-instructions.js"

type OpenCodeEventProxyState = {
  providerSessionId: string | null
}

type OpenCodeProxyDebug = (label: string, payload: unknown) => void

export async function proxyOpenCodeEventsForNativeTui(
  request: IncomingMessage,
  response: ServerResponse,
  upstreamBaseUrl: string,
  state: OpenCodeEventProxyState,
  debug: OpenCodeProxyDebug,
): Promise<void> {
  const target = new URL(request.url ?? "/", upstreamBaseUrl)
  const headers = requestHeadersForFetch(request)
  const upstream = await fetch(target, {
    method: request.method ?? "GET",
    headers,
  })

  response.statusCode = upstream.status
  upstream.headers.forEach((value, key) => {
    const lowerKey = key.toLowerCase()
    if (lowerKey !== "content-encoding" && lowerKey !== "content-length") {
      response.setHeader(key, value)
    }
  })
  if (!upstream.body) {
    response.end()
    return
  }

  let carry = ""
  const decoder = new StringDecoder("utf8")
  let refreshCounter = 0
  let refreshInFlight = false
  const refreshFromProviderSession = async () => {
    const providerSessionId = state.providerSessionId
    if (!providerSessionId || refreshInFlight || response.destroyed) return
    refreshInFlight = true
    try {
      refreshCounter = await emitNativeTranscriptRefresh({
        response,
        upstreamBaseUrl,
        sessionId: providerSessionId,
        directory: target.searchParams.get("directory"),
        counter: refreshCounter,
        debug,
      })
    } finally {
      refreshInFlight = false
    }
  }
  const refreshTimer = setInterval(() => {
    void refreshFromProviderSession().catch((error) => {
      debug("native_refresh_timer_failed", { error: formatError(error) })
    })
  }, 1_000)
  response.once("close", () => clearInterval(refreshTimer))
  try {
    for await (const chunk of Readable.fromWeb(upstream.body as never)) {
      carry += decoder.write(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk))
      while (true) {
        const separator = findSseFrameSeparator(carry)
        if (!separator) break
        const frame = carry.slice(0, separator.index)
        const delimiter = carry.slice(separator.index, separator.index + separator.length)
        const redactedFrame = redactSseFrame(frame)
        response.write(redactedFrame)
        response.write(delimiter)
        const sessionId = sessionIdNeedingNativeRefresh(frame)
        if (sessionId) {
          refreshCounter = await emitNativeTranscriptRefresh({
            response,
            upstreamBaseUrl,
            sessionId,
            directory: target.searchParams.get("directory"),
            counter: refreshCounter,
            debug,
          })
        }
        carry = carry.slice(separator.index + separator.length)
      }
    }
    if (carry) {
      response.write(redactSseFrame(carry + decoder.end()))
    }
  } finally {
    clearInterval(refreshTimer)
  }
  response.end()
}

export function requestHeadersForFetch(request: IncomingMessage): Headers {
  const headers = new Headers()
  for (const [key, value] of Object.entries(request.headers)) {
    if (!value || key.toLowerCase() === "host" || key.toLowerCase() === "content-length") continue
    if (Array.isArray(value)) {
      for (const entry of value) headers.append(key, entry)
    } else {
      headers.set(key, value)
    }
  }
  return headers
}

export function createSseHiddenInstructionRedactor(): Transform {
  let carry = ""
  const decoder = new StringDecoder("utf8")
  return new Transform({
    transform(chunk, _encoding, callback) {
      carry += decoder.write(chunk)
      while (true) {
        const separator = findSseFrameSeparator(carry)
        if (!separator) break
        const frame = carry.slice(0, separator.index)
        const delimiter = carry.slice(separator.index, separator.index + separator.length)
        this.push(redactSseFrame(frame))
        this.push(delimiter)
        carry = carry.slice(separator.index + separator.length)
      }
      callback()
    },
    flush(callback) {
      this.push(redactSseFrame(carry + decoder.end()))
      callback()
    },
  })
}

// MP-08 / MP-10 / MP-11: redact values, then encode them. Applying a prompt
// suffix regex to serialized JSON can remove its closing structure and stop
// the official native client's event reader after a real source-file read.
function redactSseFrame(frame: string): string {
  const payload = sseDataPayload(frame)
  if (!payload) return frame
  let redacted: unknown
  try { redacted = redactHiddenInstructionsFromJson(JSON.parse(payload)) }
  catch { return ": invalid upstream event omitted" }
  let emitted = false
  return frame.split(/\r?\n/).flatMap(line => {
    if (!line.startsWith("data:")) return [line]
    if (emitted) return []
    emitted = true
    return [`data: ${JSON.stringify(redacted)}`]
  }).join(frame.includes("\r\n") ? "\r\n" : "\n")
}

export function createHiddenInstructionRedactor(): Transform {
  let carry = ""
  const keepTail = 64
  return new Transform({
    transform(chunk, _encoding, callback) {
      const combined = `${carry}${chunk.toString("utf8")}`
      const redacted = redactHiddenInstructions(combined)
      const startIndex = redacted.lastIndexOf("<<<CHARIOX_NATIVE_TUI_HIDDEN_INSTRUCTIONS>>>")
      if (startIndex >= 0) {
        this.push(redacted.slice(0, startIndex))
        carry = redacted.slice(startIndex)
      } else {
        const emitLength = Math.max(0, redacted.length - keepTail)
        this.push(redacted.slice(0, emitLength))
        carry = redacted.slice(emitLength)
      }
      callback()
    },
    flush(callback) {
      this.push(redactHiddenInstructions(carry))
      callback()
    },
  })
}

function findSseFrameSeparator(value: string): { index: number; length: number } | null {
  const candidates = [
    { index: value.indexOf("\r\n\r\n"), length: 4 },
    { index: value.indexOf("\n\n"), length: 2 },
  ].filter((candidate) => candidate.index >= 0)
  if (candidates.length === 0) return null
  candidates.sort((left, right) => left.index - right.index)
  return candidates[0] ?? null
}

function sessionIdNeedingNativeRefresh(frame: string): string | null {
  const payload = sseDataPayload(frame)
  if (!payload) return null
  let event: unknown
  try {
    event = JSON.parse(payload)
  } catch {
    return null
  }
  if (!event || typeof event !== "object") return null
  const record = event as Record<string, unknown>
  const type = typeof record.type === "string" ? record.type : ""
  const properties = record.properties && typeof record.properties === "object"
    ? record.properties as Record<string, unknown>
    : {}
  const sessionId = typeof properties.sessionID === "string" ? properties.sessionID : null
  if (!sessionId) return null
  if (type === "session.idle") return sessionId
  if (type === "session.status") {
    const status = properties.status && typeof properties.status === "object"
      ? properties.status as Record<string, unknown>
      : {}
    return status.type === "idle" ? sessionId : null
  }
  return null
}

function sseDataPayload(frame: string): string | null {
  const lines = frame.split(/\r?\n/)
  const data = lines.flatMap((line) => {
    if (!line.startsWith("data:")) return []
    return [line.slice("data:".length).trimStart()]
  })
  return data.length > 0 ? data.join("\n") : null
}

async function emitNativeTranscriptRefresh(options: {
  response: ServerResponse
  upstreamBaseUrl: string
  sessionId: string
  directory: string | null
  counter: number
  debug: OpenCodeProxyDebug
}): Promise<number> {
  const url = new URL(`/session/${encodeURIComponent(options.sessionId)}/message`, options.upstreamBaseUrl)
  url.searchParams.set("limit", "100")
  if (options.directory) {
    url.searchParams.set("directory", options.directory)
  }
  const refresh = await fetch(url)
  if (!refresh.ok) {
    options.debug("native_refresh_failed", {
      sessionId: options.sessionId,
      status: refresh.status,
    })
    return options.counter
  }
  let messages: unknown
  try {
    messages = redactHiddenInstructionsFromJson(await refresh.json())
  } catch (error) {
    options.debug("native_refresh_parse_failed", {
      sessionId: options.sessionId,
      error: formatError(error),
    })
    return options.counter
  }
  if (!Array.isArray(messages)) return options.counter
  let counter = options.counter
  for (const message of messages) {
    if (!message || typeof message !== "object") continue
    const record = message as Record<string, unknown>
    if (record.info && typeof record.info === "object") {
      counter += 1
      writeSseData(options.response, {
        id: `chariox_native_refresh_${Date.now()}_${counter}`,
        type: "message.updated",
        properties: { info: record.info },
      })
    }
    if (!Array.isArray(record.parts)) continue
    for (const part of record.parts) {
      if (!part || typeof part !== "object") continue
      counter += 1
      writeSseData(options.response, {
        id: `chariox_native_refresh_${Date.now()}_${counter}`,
        type: "message.part.updated",
        properties: { part },
      })
    }
  }
  return counter
}

function writeSseData(response: ServerResponse, payload: unknown) {
  response.write(`data: ${JSON.stringify(payload)}\n\n`)
}

function formatError(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
