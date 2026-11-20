const assert = require('node:assert/strict')
const { mkdtemp, writeFile, rm } = require('node:fs/promises')
const { readFileSync } = require('node:fs')
const { tmpdir } = require('node:os')
const { join } = require('node:path')
const { runInNewContext } = require('node:vm')
const { once } = require('node:events')
const test = require('node:test')

async function startTrace(t) {
  const work = await mkdtemp(join(tmpdir(), 'loadlab-permissions-'))
  t.after(() => rm(work, { recursive: true, force: true }))
  const rows = [
    { v: 1, source: 'generator', seed: 42, agent: 'probe', notes: 'permission replay' },
    { t: 0, kind: 'prompt', chars: 20 },
    { t: 0, kind: 'permission', tool: 'Bash' },
    { t: 0, kind: 'permission', tool: 'Bash' },
    { t: 0, kind: 'end', outcome: 'completed' },
  ]
  await writeFile(join(work, 'probe.jsonl'), rows.map(row => JSON.stringify(row)).join('\n') + '\n')
  const directory = join(__dirname, '../../../apps/kernel/src/provider/registry/dev_stub_adapter')
  const source = ['runtime_mcp_client.cjs', 'trace_runtime.cjs', 'trace_server.cjs']
    .map(name => readFileSync(join(directory, name), 'utf8')).join('\n')
  const { parseTrace, filler } = require(join(directory, 'trace_format.cjs'))
  const server = runInNewContext(source, {
    require, parseTrace, filler, Buffer, AbortController, performance, URL,
    process: { argv: ['node', '0'], env: { CHARIOX_LOADLAB_TRACE_DIR: work, CHARIOX_LOADLAB_TRACE_AGENT: 'probe' } },
  })
  t.after(() => { server.closeAllConnections(); server.close() })
  await once(server, 'listening')
  const url = `http://127.0.0.1:${server.address().port}`
  const send = (method, path, value) => fetch(url + path, {
    method, body: JSON.stringify(value), signal: AbortSignal.timeout(3000),
  })
  const stream = await fetch(url + '/event', { signal: AbortSignal.timeout(5000) })
  const lines = require('node:readline').createInterface({ input: require('node:stream').Readable.fromWeb(stream.body) })
  t.after(() => lines.close())
  const events = lines[Symbol.asyncIterator]()
  return {
    send,
    async nextEvent() {
      for (;;) {
        const line = await events.next()
        assert.ok(!line.done, 'trace event stream ended before the turn settled')
        if (line.value.startsWith('data: ')) return JSON.parse(line.value.slice(6))
      }
    },
  }
}

for (const mode of ['create', 'update', 'session grant']) {
  test(`trace applies the ${mode} permission before asking`, { timeout: 6000 }, async t => {
    const trace = await startTrace(t)
    const rules = [{ permission: 'bash', pattern: '*', action: 'allow' }]
    const session = await (await trace.send('POST', '/session', mode === 'create' ? { permission: rules } : {})).json()
    const path = `/session/${session.id}`
    if (mode === 'update') assert.equal((await trace.send('PATCH', path, { permission: rules })).status, 200)
    assert.equal((await trace.send('POST', path + '/prompt_async', { messageID: 'prompt' })).status, 204)

    let prompts = 0
    for (;;) {
      const { type, properties } = await trace.nextEvent()
      if (type === 'permission.asked') {
        prompts++
        assert.equal(prompts, mode === 'session grant' ? 1 : 0)
        const reply = await trace.send('POST', path + '/permissions/' + properties.id, { response: 'always' })
        assert.equal(reply.status, 200)
      }
      if (type === 'session.status' && properties.status.type === 'idle') break
    }
    assert.equal(prompts, mode === 'session grant' ? 1 : 0)
    const messages = await (await trace.send('GET', path + '/message')).json()
    assert.equal(messages[0].info.finish, 'stop')
    assert.equal(messages[0].info.error, undefined)
  })
}
