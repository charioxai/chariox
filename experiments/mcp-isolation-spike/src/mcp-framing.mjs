import { once } from 'node:events'

export class JsonRpcFramer {
  constructor(input, onMessage, { onFrame = () => {} } = {}) {
    this.input = input
    this.onMessage = onMessage
    this.onFrame = onFrame
    this.buffer = Buffer.alloc(0)
    this.closed = false
    input.on('data', (chunk) => this.push(chunk))
    input.on('end', () => { this.closed = true })
  }

  push(chunk) {
    this.buffer = Buffer.concat([this.buffer, Buffer.from(chunk)])
    while (true) {
      const crlfHeaderEnd = this.buffer.indexOf('\r\n\r\n')
      const lfHeaderEnd = this.buffer.indexOf('\n\n')
      let headerEnd = crlfHeaderEnd
      let delimiterLength = 4
      if (headerEnd < 0 || (lfHeaderEnd >= 0 && lfHeaderEnd < headerEnd)) {
        headerEnd = lfHeaderEnd
        delimiterLength = 2
      }

      if (headerEnd < 0) {
        const lineEnd = this.buffer.indexOf('\n')
        if (lineEnd < 0) return
        const line = this.buffer.subarray(0, lineEnd).toString('utf8').trim()
        if (!line) {
          this.buffer = this.buffer.subarray(lineEnd + 1)
          continue
        }
        if (!line.startsWith('{')) return
        this.buffer = this.buffer.subarray(lineEnd + 1)
        this.onFrame('line')
        this.onMessage(JSON.parse(line))
        continue
      }

      const header = this.buffer.subarray(0, headerEnd).toString('utf8')
      const lengthMatch = /^content-length:\s*(\d+)$/im.exec(header)
      if (!lengthMatch) {
        throw new Error(`missing Content-Length in MCP frame header: ${JSON.stringify(header)}`)
      }
      const bodyLength = Number(lengthMatch[1])
      const bodyStart = headerEnd + delimiterLength
      const frameLength = bodyStart + bodyLength
      if (this.buffer.length < frameLength) return
      const body = this.buffer.subarray(bodyStart, frameLength).toString('utf8')
      this.buffer = this.buffer.subarray(frameLength)
      const message = JSON.parse(body)
      this.onFrame('content-length')
      this.onMessage(message)
    }
  }
}

export function writeJsonRpc(output, message, frameFormat = 'line') {
  const body = JSON.stringify(message)
  if (frameFormat === 'content-length') {
    output.write(`Content-Length: ${Buffer.byteLength(body, 'utf8')}\r\n\r\n${body}`)
    return
  }
  output.write(`${body}\n`)
}

export async function waitForExit(child) {
  if (!child || child.exitCode != null) return child?.exitCode ?? 0
  const [code] = await once(child, 'exit')
  return code ?? 0
}
