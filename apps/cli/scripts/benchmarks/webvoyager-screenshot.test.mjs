// MP-08/MP-10: retain the first failing CDP seam, rather than a generic capture error.
import test from 'node:test'
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import vm from 'node:vm'

test('screenshot error retains method/code and releases attached sessions', async () => {
  const messages = []
  class Socket {
    constructor() { queueMicrotask(() => this.onopen()) }
    send(raw) {
      const request = JSON.parse(raw); messages.push(request)
      let result = {}
      if (request.method === 'Target.getTargets') result = { targetInfos: [{ type: 'page', targetId: 'owned-page', url: 'https://public.invalid' }] }
      if (request.method === 'Target.attachToTarget') result = { sessionId: 'owned-session' }
      if (request.method === 'Runtime.evaluate') result = { result: { value: JSON.stringify({ visible: true, focused: true }) } }
      queueMicrotask(() => this.onmessage({ data: JSON.stringify({ id: request.id,
        ...(request.method === 'Page.captureScreenshot' ? { error: { code: -32000, message: 'Unable to capture screenshot' } } : { result }) }) }))
    }
    close() { this.closed = true }
  }
  const source = (await readFile(new URL('./webvoyager-screenshot.mjs', import.meta.url), 'utf8'))
    .replace(/import \{ writeFile \} from 'node:fs\/promises'/, 'const writeFile = async () => { throw Error("unexpected file write") }')
  const execution = vm.runInNewContext(`(async () => {${source}\n})()`, {
    process: { argv: ['node', 'script', '/tmp/benchwv-screenshot1.png'] },
    fetch: async () => ({ json: async () => ({ Browser: 'Chromium-test', webSocketDebuggerUrl: 'ws://owned.invalid' }) }),
    WebSocket: Socket, queueMicrotask, setTimeout, clearTimeout, console, Buffer,
  })
  await assert.rejects(execution, error => error.method === 'Page.captureScreenshot' && error.code === -32000)
  assert(messages.some(message => message.method === 'Target.detachFromTarget' && message.params.sessionId === 'owned-session'))
})
