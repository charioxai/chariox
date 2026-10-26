// MP-11 #879: private dev-stub tool execution through the existing kernel terminal path.
import { randomUUID } from 'node:crypto'
import { setTimeout as sleep } from 'node:timers/promises'
export const PROVIDER_MCP_FIXTURE_MODEL = 'runtime-mcp-fixture'

export function bindProviderMcpFixture(run, { client, requests, sessionId, attachmentId }) {
  if (run.model !== PROVIDER_MCP_FIXTURE_MODEL || run.provider !== 'dev-stub') {
    throw new Error('MCP drill requires the explicit private dev-stub fixture')
  }
  const fixture = new ProviderMcpFixture({ client, requests, sessionId, attachmentId, runId: run.id })
  return Object.defineProperty({ ...run }, 'fixtureMcp', { value: fixture })
}

class ProviderMcpFixture {
  #context
  #proxy
  constructor(context, proxy = null) { this.#context = context; this.#proxy = proxy }
  proxy(name) { return new ProviderMcpFixture(this.#context, name) }
  toJSON() { return { kind: 'private-provider-mcp-fixture' } }
  async call(method, params = {}, { timeoutMs = 95000, pollMs = 25, rawEnvelope = false } = {}) {
    const { client, requests, sessionId, attachmentId, runId } = this.#context
    const id = randomUUID()
    let text = ''
    let outputError = null
    const deadline = Date.now() + timeoutMs
    const bounded = async promise => {
      let timer
      try {
        return await Promise.race([Promise.resolve(promise), new Promise((_, reject) => {
          timer = setTimeout(() => reject(Error('private provider MCP fixture timed out')), Math.max(1, deadline - Date.now()))
        })])
      } finally { clearTimeout(timer) }
    }
    const seen = new Set()
    const receive = records => {
      for (const record of records ?? []) {
        if (record.provider_run_id !== runId || record.kind === 'PromptEcho') continue
        if (record.record_id !== undefined) {
          if (seen.has(record.record_id)) continue
          seen.add(record.record_id)
        }
        text += Array.isArray(record.bytes) ? Buffer.from(record.bytes).toString('utf8') : String(record.text ?? '')
        if (text.length > 16 * 1024 * 1024) throw Error('private fixture output exceeded limit')
      }
    }
    const unsubscribe = client.onKernelEvent?.(event => {
      try { if (event.event === 'terminal_output') receive(event.records) } catch (error) { outputError = error }
    })
    try {
      // Subscribe before input so another observer's pump cannot consume the result.
      await bounded(client.subscribeToKernelEvents?.(sessionId, attachmentId))
      const input = `\nMP11_MCP_REQUEST ${id} ${Buffer.from(JSON.stringify({ method, params, proxy: this.#proxy })).toString('base64')}\n`
      if (input.length > 1024 * 1024) throw Error('private fixture request exceeded limit')
      await bounded(client.send(requests.sendTerminalInputRequest(sessionId, attachmentId, input, runId)))
      while (Date.now() < deadline) {
        const response = await bounded(client.send(requests.pumpTerminalOutputRequest(sessionId, attachmentId)))
        if (outputError) throw outputError
        receive((response.TerminalOutput ?? response.TerminalOutputPumped)?.records)
        const match = text.match(new RegExp(`MP11_MCP_RESULT ${id} ([A-Za-z0-9+/=]+)[\\r\\n]`))
        if (match) {
          const envelope = JSON.parse(Buffer.from(match[1], 'base64').toString('utf8'))
          if (rawEnvelope) return envelope
          if (envelope.error) throw Error(`runtime MCP ${method} failed: ${JSON.stringify(envelope.error)}`)
          return envelope.result
        }
        await sleep(pollMs)
      }
      throw Error('private provider MCP fixture timed out')
    } finally { unsubscribe?.() }
  }
}
