// Minimal OpenCode server surface used by the kernel's managed runtime.
const { createServer } = require('node:http')
const { readFileSync } = require('node:fs')
const { join } = require('node:path')

const traceAgent = process.env.CHARIOX_LOADLAB_TRACE_AGENT
const traceRoot = process.env.CHARIOX_LOADLAB_TRACE_DIR
function loadTrace(agent) {
  if (!/^[a-zA-Z0-9_-]{1,64}$/.test(agent ?? '')) throw new Error('invalid load lab agent alias')
  const file = join(traceRoot, `${agent}.jsonl`)
  const plan = parseTrace(readFileSync(file, 'utf8'), file)
  if (plan.header.agent !== agent) throw new Error(`${file}: line 1: agent does not match the filename`)
  return plan
}

let trace
let traceError
try {
  trace = loadTrace(traceAgent)
} catch (error) {
  traceError = error.message
}

const sessions = new Map()
const subscribers = new Set()
const connected = new Set()
const permissions = new Map()
let nextTurn = 0

function event(type, properties) {
  for (const response of subscribers) response.write(`data: ${JSON.stringify({ type, properties })}\n\n`)
}

function status(session, type) {
  session.status = { type }
  event('session.status', { sessionID: session.id, status: session.status })
}

function projectTurn(session, parentID) {
  const info = { id: randomUUID(), sessionID: session.id, role: 'assistant', parentID }
  const message = { info, parts: [] }
  session.messages.push(message)
  event('message.updated', { info })
  return {
    part(type, fields) {
      const part = { id: randomUUID(), sessionID: session.id, messageID: info.id, type, ...fields }
      message.parts.push(part)
      event('message.part.updated', { part })
      return part
    },
    update(part, fields) {
      Object.assign(part, fields)
      event('message.part.updated', { part })
    },
    async stream(kind, text, duration, signal) {
      if (!duration) {
        this.part(kind, { text, time: { end: Date.now() } })
        return
      }
      const part = this.part(kind, { text: '' })
      const count = Math.min(text.length, Math.ceil(duration / 50))
      const start = performance.now()
      for (let index = 0; index < count; index++) {
        const delta = text.slice(Math.floor(index * text.length / count), Math.floor((index + 1) * text.length / count))
        const wait = (index + 1) * duration / count - (performance.now() - start)
        if (wait > 0) await delay(wait, undefined, { signal })
        part.text += delta
        event('message.part.delta', { sessionID: session.id, messageID: info.id, partID: part.id, field: 'text', delta })
      }
      this.update(part, { time: { end: Date.now() } })
    },
    async permission(name, signal) {
      const id = randomUUID()
      const answered = new Promise((resolve, reject) => {
        permissions.set(id, resolve)
        signal.addEventListener('abort', () => reject(signal.reason), { once: true })
      })
      event('permission.asked', {
        id, sessionID: session.id, permission: name.toLowerCase(), patterns: ['generated sample'],
        metadata: { command: 'printf generated-sample' },
        tool: { name, messageID: info.id },
      })
      try {
        await answered
      } finally {
        permissions.delete(id)
      }
    },
    finish(outcome, diagnostic) {
      info.finish = 'stop'
      info.time = { completed: Date.now() }
      if (outcome !== 'completed') {
        info.error = {
          name: outcome === 'cancelled' ? 'MessageAbortedError' : 'UnknownError',
          data: { message: diagnostic },
        }
        event('session.error', { sessionID: session.id, error: info.error })
      }
      event('message.updated', { info })
    },
  }
}

async function run(session, parentID) {
  const active = new AbortController()
  session.active = active
  status(session, 'busy')
  const output = projectTurn(session, parentID)
  try {
    if (traceError) throw new Error(traceError)
    const turn = trace.turns[nextTurn++]
    if (!turn) throw new Error('generated trace has no remaining turns')
    const outcome = await replaySteps(turn, output, active.signal)
    output.finish(outcome, outcome === 'cancelled' ? 'Generated turn cancelled' : 'Generated turn failed')
  } catch (error) {
    output.finish(active.signal.aborted ? 'cancelled' : 'error', active.signal.aborted ? 'Generated turn cancelled' : error.message)
  } finally {
    session.active = undefined
    status(session, 'idle')
  }
}

function json(response, value, code = 200) {
  const body = JSON.stringify(value)
  response.writeHead(code, { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(body) })
  response.end(body)
}

async function body(request) {
  let data = ''
  for await (const chunk of request) {
    data += chunk
    if (data.length > 4 * 1024 * 1024) throw new Error('request too large')
  }
  return JSON.parse(data)
}

async function route(request, response) {
  const url = new URL(request.url, 'http://localhost')
  const path = url.pathname
  const method = request.method
  if (method === 'GET' && path === '/global/health') return json(response, { healthy: true, version: 'loadlab' })
  if (method === 'GET' && path === '/event') {
    response.useChunkedEncodingByDefault = false
    response.writeHead(200, { 'Content-Type': 'text/event-stream', Connection: 'close' })
    response.write('data: {"type":"server.connected","properties":{}}\n\n')
    subscribers.add(response)
    response.on('close', () => subscribers.delete(response))
    return
  }
  if (method === 'GET' && path === '/mcp') {
    return json(response, Object.fromEntries([...connected].map(name => [name, { status: 'connected' }])))
  }
  const mcp = path.match(/^\/mcp\/([^/]+)\/connect$/)
  if (method === 'POST' && mcp) {
    connected.add(mcp[1])
    return json(response, true)
  }
  if (method === 'GET' && path === '/session/status') {
    return json(response, Object.fromEntries([...sessions].map(([id, session]) => [id, session.status])))
  }
  if (method === 'POST' && path === '/session') {
    const id = randomUUID()
    sessions.set(id, { id, status: { type: 'idle' }, messages: [] })
    return json(response, { id })
  }
  const [, base, sessionId, operation, permission] = path.split('/')
  const session = base === 'session' && sessions.get(sessionId)
  if (!session) return json(response, { error: 'unknown load lab route' }, 404)
  if (method === 'PATCH' && !operation) return json(response, { id: session.id })
  if (method === 'GET' && operation === 'message') {
    return json(response, session.messages.slice(-Number(url.searchParams.get('limit') || 50)))
  }
  if (method === 'POST' && operation === 'abort') {
    session.active?.abort()
    return json(response, true)
  }
  if (method === 'POST' && operation === 'permissions' && permissions.has(permission)) {
    permissions.get(permission)((await body(request)).response)
    return json(response, true)
  }
  if (method === 'POST' && operation === 'prompt_async') {
    if (session.active) return json(response, { error: 'generated turn already active' }, 409)
    const payload = await body(request)
    if (typeof payload.messageID !== 'string') return json(response, { error: 'missing prompt id' }, 400)
    response.writeHead(204).end()
    void run(session, payload.messageID)
    return
  }
  return json(response, { error: 'unknown load lab route' }, 404)
}

createServer((request, response) => {
  route(request, response).catch(() => json(response, { error: 'invalid load lab request' }, 400))
}).listen(Number(process.argv[1]), '127.0.0.1')
