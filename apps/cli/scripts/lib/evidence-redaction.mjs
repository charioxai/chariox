import { StringDecoder } from "node:string_decoder"
import { Transform } from "node:stream"

const controlCharacters = /[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g
const sensitivePatterns = [
  /\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]+/gi,
  /\b([A-Za-z0-9_]*(?:token|secret|password|credential|cookie|authorization|api[_-]?key)[A-Za-z0-9_]*)\s*[=:]\s*([^\s,;]+)/gi,
  /(?<![a-z0-9+.-])([a-z][a-z0-9+.-]*:\/\/[^\s/:@]+:)[^\s/@]+(@)/gi,
]

function secrets(values) {
  return [...new Set(values.filter(value => typeof value === "string" && value.length >= 4))]
    .sort((a, b) => b.length - a.length)
}

export function redactText(value, secretValues = []) {
  let result = String(value ?? "").replace(controlCharacters, "")
  for (const secret of secrets(secretValues)) result = result.split(secret).join("[REDACTED]")
  return result
    .replace(sensitivePatterns[0], "$1 [REDACTED]")
    .replace(sensitivePatterns[1], "$1=[REDACTED]")
    .replace(sensitivePatterns[2], "$1[REDACTED]$2")
}

// Keep incomplete identifiers, authorization headers and URLs, including ones
// longer than the normal overlap. A later chunk may turn any of them into a
// sensitive match. Complete matches must also be consumed as original spans.
const pendingPatterns = [
  /(?<![a-z0-9+.-])[a-z][a-z0-9+.-]*$/gi,
  /(?<![A-Za-z0-9_])[A-Za-z0-9_]+\s*(?:[=:]\s*[^\s,;]*)?$/g,
  /\b(?:Bearer|Basic)\s+[A-Za-z0-9._~+/=-]*$/gi,
  /(?<![a-z0-9+.-])[a-z][a-z0-9+.-]*:(?:\/?\/?[^\s]*)?$/gi,
]

function safeCut(buffered, overlap, secretValues) {
  let cut = Math.max(0, buffered.length - overlap)
  const spans = []
  for (const secret of secretValues) {
    let index = buffered.indexOf(secret)
    while (index !== -1) {
      spans.push([index, index + secret.length])
      index = buffered.indexOf(secret, index + 1)
    }
  }
  for (const pattern of sensitivePatterns) {
    for (const match of buffered.matchAll(pattern)) spans.push([match.index, match.index + match[0].length])
  }
  for (const pattern of pendingPatterns) {
    for (const match of buffered.matchAll(pattern)) cut = Math.min(cut, match.index)
  }
  // Overlapping matches can move the cut into an earlier match.
  let previous
  do {
    previous = cut
    if (cut > 0 && /[\uD800-\uDBFF]/.test(buffered[cut - 1]) && /[\uDC00-\uDFFF]/.test(buffered[cut])) cut--
    for (const [start, end] of spans) if (start < cut && cut < end) cut = start
  } while (cut !== previous)
  return cut
}

export function redactingTransform({ secretValues = [], maximumBytes }) {
  const values = secrets(secretValues)
  const overlap = Math.max(512, ...values.map(value => value.length))
  const maximumPending = Math.max(65_536, overlap * 2)
  const decoder = new StringDecoder("utf8")
  let buffered = ""
  let discardingLine = false
  let retained = 0
  let truncated = false
  const marker = Buffer.from("\n[REDACTED LOG TRUNCATED]\n")
  const contentLimit = maximumBytes - marker.length
  const emit = (stream, sanitized) => {
    const encoded = Buffer.from(sanitized)
    const emitted = encoded.subarray(0, Math.max(0, contentLimit - retained))
    retained += emitted.length
    if (emitted.length) stream.push(emitted)
    if (emitted.length < encoded.length) truncated = true
  }
  const consume = (stream, text) => {
    // Process bounded pieces even when a producer hands us a very large chunk.
    for (let start = 0; start < text.length; start += 16_384) {
      let piece = text.slice(start, start + 16_384).replace(controlCharacters, "")
      if (discardingLine) {
        const newline = piece.indexOf("\n")
        if (newline === -1) continue
        piece = piece.slice(newline + 1)
        discardingLine = false
      }
      buffered += piece
      const cut = safeCut(buffered, overlap, values)
      if (cut) {
        emit(stream, redactText(buffered.slice(0, cut), values))
        buffered = buffered.slice(cut)
      }
      // Never retain an unbounded partial sensitive record or expose its tail.
      // Discard the entire pending line and its continuation on overflow.
      if (buffered.length > maximumPending) {
        emit(stream, "[REDACTED OVERSIZED RECORD]\n")
        buffered = ""
        discardingLine = true
      }
    }
  }
  return new Transform({
    transform(chunk, _encoding, callback) {
      consume(this, decoder.write(chunk))
      callback()
    },
    flush(callback) {
      consume(this, decoder.end())
      emit(this, redactText(buffered, values))
      if (truncated) this.push(marker)
      callback()
    },
  })
}
