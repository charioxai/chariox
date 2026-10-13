// MP-08 / MP-11: exercise the shipped capture helper without starting a drill.
import assert from 'node:assert/strict'
import test from 'node:test'
import vm from 'node:vm'
import path from 'node:path'
import {readFile} from 'node:fs/promises'
const source = await readFile(new URL('./live-byom-account-drill.mjs', import.meta.url), 'utf8')
const shotSource = source.match(/^const shot = .*$/m)[0]
test('MP-08 / MP-11 every diagnostic screenshot masks capability-bearing controls', async () => {
  const capabilityControls = {selector: 'controls'}, calls = [], styles = []
  const page = {locator: () => capabilityControls, addStyleTag: async options => {styles.push(options); return {evaluate: async () => {}}}, screenshot: async options => {calls.push(options)}}
  const shot = vm.runInNewContext(shotSource + '; shot', {path, E: '/evidence', report: {}, safe: String})
  await shot(page, 'failure-0')
  assert.deepEqual(Array.from(calls[0].mask ?? []), [capabilityControls])
  assert.match(styles[0]?.content ?? "", /visibility: hidden !important/)
  assert.match(source, /catch \(error\) \{[^]*await shot\(open, `failure-\$\{index\}`\)/)
})

test('MP-08 / MP-11 closing one of two control sockets cannot certify sign-out', async () => {
  const handlers = new Map(), page = {on: (event, fn) => handlers.set(event, fn)}
  const newPage = vm.runInNewContext(source.match(/async function newPage\(context\) \{[^]*?\n\}\nasync function enroll/)[0].replace(/\nasync function enroll$/, '') + '; newPage', {
    network: {attach: () => {}}, report: {}, relay: 'wss://relay.example.test', URL, Date, Map,
  })
  await newPage({newPage: async () => page})
  const sockets = [0, 1].map(id => {const listeners = new Map(); return {url: () => 'wss://relay.example.test/?lane=control', on: (event, fn) => listeners.set(event, fn), close: () => listeners.get('close')?.(), id}})
  const event = (state, client) => ({text: () => `[chariox:kernel-transport] relay_websocket_${state}`, args: () => [null, {evaluate: async fn => fn({lane: 'control', client, relayUrl: 'wss://relay.example.test'})}]})
  for (const socket of sockets) {handlers.get('websocket')?.(socket); handlers.get('console')?.(event('connected', socket.id))}
  await new Promise(resolve => setImmediate(resolve))
  const open = () => page.openRelayConnections?.() ?? page.openRelayLanes()
  assert.equal(open().length, 2, 'each control WebSocket is observed separately')
  sockets[0].close(); handlers.get('console')?.(event('closed', 0))
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(open().length, 1, 'the surviving control socket keeps sign-out RED')
  sockets[1].close()
  assert.equal(open().length, 0)
})
