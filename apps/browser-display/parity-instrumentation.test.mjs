// MP-08 / MP-10 / MP-11: supplementary checks for paired cadence and metadata-only instrumentation.
import assert from 'node:assert/strict'
import test from 'node:test'
import http from 'node:http'
import net from 'node:net'
import { parityCadence } from './parity-cadence.mjs'
import { timingMux } from './parity-timing-mux.mjs'

test('MP-10: both arms reproduce the recorded campaign cadence', () => {
  const s = parityCadence(1, 'mdn', 'click'), m = parityCadence(1, 'mdn', 'click')
  const gaps = Array.from({ length: 20 }, () => s.next())
  assert.deepEqual(gaps, Array.from({ length: 20 }, () => m.next()))
  assert.ok(gaps.every(gap => gap >= 1200 && gap < 1700))
  assert.ok(new Set(gaps).size === 20)
  assert.deepEqual(s.record(), m.record())
  assert.equal(parityCadence(1, 'mdn', 'click', 'fixed').next(), 1400)
  assert.equal(parityCadence(1, 'mdn', 'type', 'fixed').next(), 1000)
})

test('MP-11: mux forwards both arms but records only numeric metadata and client class', async t => {
  const rows = [], sockets = new Set()
  const endpoint = () => {
    const server = http.createServer()
    server.on('upgrade', (_req, socket) => {
      sockets.add(socket)
      socket.once('close', () => sockets.delete(socket))
      socket.write('HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n')
      socket.on('data', chunk => socket.write(chunk))
    })
    return server
  }
  const relay = endpoint(), baseline = endpoint()
  await Promise.all([relay, baseline].map(server => new Promise(resolve => server.listen(0, '127.0.0.1', resolve))))
  const mux = timingMux({ relay: relay.address().port, baseline: baseline.address().port, record: row => rows.push(row) })
  t.after(async () => { await mux.close(); for (const socket of sockets) socket.destroy(); await Promise.all([relay, baseline].map(server => new Promise(resolve => server.close(resolve)))) })
  await new Promise(resolve => mux.server.listen(0, '127.0.0.1', resolve))
  for (const path of ['/relay?token=synthetic-private-marker', '/api/websockets?token=synthetic-private-marker']) {
    const socket = net.connect(mux.server.address().port, '127.0.0.1')
    t.after(() => socket.destroy())
    await new Promise((resolve, reject) => {
      socket.once('error', reject)
      socket.once('data', () => resolve())
      socket.write(`GET ${path} HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nUser-Agent: Mozilla\r\nAuthorization: synthetic-private-marker\r\n\r\n`)
    })
    const echoed = new Promise((resolve, reject) => { socket.once('data', resolve); socket.once('error', reject) })
    socket.write('synthetic-private-marker')
    assert.equal((await echoed).toString(), 'synthetic-private-marker')
    socket.destroy()
  }
  assert.deepEqual(rows.filter(row => row.open).map(row => row.path), [0, 1])
  assert.ok(rows.filter(row => row.dir === 'up').length >= 2)
  const allowed = new Set(['at', 'conn', 'path', 'ua', 'open', 'dir', 'bytes', 'closed'])
  assert.ok(rows.every(row => Object.keys(row).every(key => allowed.has(key))))
  assert.ok(!JSON.stringify(rows).includes('private-marker'))
  assert.throws(() => timingMux({ relay: 1, baseline: 1, record() {} }), /owned loopback port/)
})
