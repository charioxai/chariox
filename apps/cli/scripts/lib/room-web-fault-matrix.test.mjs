// MP-08 / MP-10: exercise recovery ordering through the real fault matrix/control files.
// Browser, kernel and TUI responses are synthetic; this is not live acceptance.
import assert from 'node:assert/strict'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { setTimeout as sleep } from 'node:timers/promises'
import { createRoomWebFaultControl } from './room-web-fault-control.mjs'
import { runRoomWebFaultMatrix } from './room-web-fault-matrix.mjs'

async function recoveryFixture(delayedSide, registerAttachment = true) {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'chariox-b215-webfault-recovery-'))
  const previousWindow = globalThis.window
  const state = {
    environment: { session_id: 'room', environment_id: 'env', runtime_generation: 1,
      viewport: { revision: 1 }, tabs: [{ tab_id: 'tab' }] },
    page: { actions: [] }, attachmentIds: ['local-old', 'remote-old'],
  }
  const clients = Object.fromEntries(['local', 'remote'].map(side => [side, {
    sessionId: 'room', attachmentId: `${side}-old`, daemonDisconnected: false,
  }]))
  const observations = []
  let stopped = false
  let restarted = false
  let recoveryPolls = 0
  let disposed = false
  let polling = true
  let pumpLoop
  try {
    await writeFile(path.join(directory, 'fault-selection.json'), JSON.stringify({ faults: ['kernel'], boundaries: ['beginning'] }))
    const pump = createRoomWebFaultControl({ directory, handlers: {
      state() {
        const snapshot = structuredClone(state)
        observations.push({ kind: 'state', restarted, attachmentIds: snapshot.attachmentIds })
        return snapshot
      },
      clients() {
        if (restarted && ++recoveryPolls === 2) {
          clients[delayedSide] = { sessionId: 'room', attachmentId: `${delayedSide}-new`, daemonDisconnected: false }
          if (registerAttachment) state.attachmentIds.push(`${delayedSide}-new`)
        }
        const snapshot = structuredClone(clients)
        observations.push({ kind: 'clients', recovered: restarted && recoveryPolls >= 2 })
        return snapshot
      },
      kernel_stop() {
        stopped = true
        for (const client of Object.values(clients)) client.daemonDisconnected = true
        return { stopped: true }
      },
      kernel_start() {
        stopped = false
        restarted = true
        const otherSide = delayedSide === 'local' ? 'remote' : 'local'
        clients[otherSide] = { sessionId: 'room', attachmentId: `${otherSide}-new`, daemonDisconnected: false }
        state.attachmentIds = [`${otherSide}-new`]
        state.environment.runtime_generation++
        return { started: true }
      },
      // Submission can succeed while the slash command itself only reports a footer error.
      room_status() { return { submitted: true } },
    } })
    pumpLoop = (async () => {
      while (polling) { await pump(); await sleep(10) }
    })()
    const projection = () => ({ timeOrigin: 1, shellMounted: true, shellSame: true,
      environmentStatus: stopped ? 'stale' : 'ready', frameInputReady: stopped ? 'false' : 'true', inputEnabled: stopped ? 'false' : 'true' })
    const result = await runRoomWebFaultMatrix({
      page: { async evaluate() { return projection() }, async screenshot() {}, locator() { return { async waitFor() {} } } },
      client: {
        onKernelEvent() { return () => { disposed = true } }, async connect() {},
        async send(request) {
          if (request.attach) return { SessionAttached: {} }
          state.page.actions.push({ action_id: 'operation', idempotency_key: request.key, state: 'completed', sequence: 1 })
          return { accepted: true }
        },
      },
      ready: { sessionId: 'room' }, coordinationDir: directory, evidenceRoot: directory,
      setIdentityMode() {}, requests: {
        attachToSessionRequest() { return { attach: true } },
        submitRoomEnvironmentActionRequest(_session, _generation, _revision, key) { return { key } },
      },
    })
    const report = JSON.parse(await readFile(result.reportPath, 'utf8'))
    assert.equal(disposed, true, 'observer must be disposed')
    assert.ok(observations.some(value => value.kind === 'state' && value.restarted
      && !value.attachmentIds.includes(`${delayedSide}-new`)), 'fixture must read kernel state before the late TUI reattaches')
    return { result, row: report.rows[0], observations }
  } finally {
    polling = false
    try { await pumpLoop } finally {
      if (previousWindow === undefined) delete globalThis.window
      else globalThis.window = previousWindow
      await rm(directory, { recursive: true, force: true })
    }
  }
}

for (const side of ['local', 'remote']) {
  test(`MP-08 / MP-10 refresh kernel membership after the ${side} TUI reattaches during recovery`, async () => {
    const { result, row, observations } = await recoveryFixture(side)
    assert.deepEqual(row.errors, [])
    assert.equal(result.green, 1)
    assert.equal(row.clientsRecovery[side].attachmentId, `${side}-new`)
    assert.ok(observations.findLastIndex(value => value.kind === 'state')
      > observations.findLastIndex(value => value.kind === 'clients' && value.recovered), 'kernel membership must be sampled after TUI recovery')
  })
}

test('MP-08 / MP-10 refreshed membership still rejects a connected TUI absent from the kernel', async () => {
  const { result, row } = await recoveryFixture('remote', false)
  assert.equal(result.red, 1)
  assert.deepEqual(row.errors, ['TUI attachment is absent from kernel state'])
})
