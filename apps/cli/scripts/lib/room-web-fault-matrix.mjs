// MP-08 / MP-10: actual Cloud Web rendering plus both product TUIs.
// Host operations use the producer's private lane-owned fault control channel.
import assert from 'node:assert/strict'
import { createHmac, randomUUID } from 'node:crypto'
import { readFile, rename, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'
import { assertWebFaultRecovery, controllerFaultAttributed } from './room-web-fault-invariants.mjs'

export function faultRelayCredential(token, ready, mode) {
  if (mode === 'valid') return token
  const [prefix, payload] = token.split('.')
  const claims = JSON.parse(Buffer.from(payload, 'base64url'))
  if (mode === 'expired') claims.expires_at_ms = Date.now() - 1000
  else if (mode === 'stale-identity') claims.realm_id = 'webfault-stale-realm'
  else throw new Error('unsupported identity fault')
  const encoded = Buffer.from(JSON.stringify(claims)).toString('base64url')
  return `${prefix}.${encoded}.${createHmac('sha256', ready.relayScopedSecret).update(encoded).digest('base64url')}`
}

export async function runRoomWebFaultMatrix({ page, client, ready, coordinationDir, evidenceRoot, setIdentityMode, identityFaultStats, requests }) {
  // The companion's observational client runs under Node; product browser timers remain native.
  globalThis.window ??= { setTimeout, clearTimeout, setInterval, clearInterval }
  const attempt = randomUUID()
  let sequence = await readFile(path.join(coordinationDir, 'fault-response.json'), 'utf8').then(JSON.parse).then(value => value.id).catch(() => 0)
  let latestEnvironment = null
  const rows = []
  const reportPath = path.join(evidenceRoot, 'MP-08-MP-10-webfault-matrix.json')
  const report = async () => writeFile(reportPath, JSON.stringify({ schema: 'chariox.webfault_matrix.v1', mpItems: ['MP-08', 'MP-10'], scope: 'local provider-free signed 686; no hosted/fresh-machine or official-provider acceptance', rows }, null, 2), { mode: 0o600 })
  async function control(operation, timeoutMs = 65000) {
    const id = ++sequence
    const temporary = path.join(coordinationDir, 'fault-command.tmp')
    await writeFile(temporary, JSON.stringify({ id, operation }), { mode: 0o600 })
    await rename(temporary, path.join(coordinationDir, 'fault-command.json'))
    const deadline = Date.now() + timeoutMs
    while (Date.now() < deadline) {
      const response = await readFile(path.join(coordinationDir, 'fault-response.json'), 'utf8').then(JSON.parse).catch(() => null)
      if (response?.id === id) {
        assert.equal(response.ok, true, `${operation}: ${response.error ?? 'failed'}`)
        return response.data
      }
      await sleep(100)
    }
    throw new Error(`fault control ${operation} timed out`)
  }
  async function webSnapshot(label) {
    const projection = await page.evaluate(() => {
      const node = selector => document.querySelector(selector)
      const attr = (selector, key) => node(selector)?.getAttribute(key) ?? null
      const text = selector => node(selector)?.textContent?.trim().slice(0, 1800) ?? null
      return {
        timeOrigin: performance.timeOrigin,
        shellMounted: window.__webfaultShell?.isConnected === true,
        shellSame: window.__webfaultShell === node('.terminal-app-workspace'),
        kernelStatus: text('[data-terminal-status-root]'),
        connection: attr('.view-connection-state', 'data-state'),
        environmentStatus: attr('[data-room-environment-status]', 'data-room-environment-status'),
        inputEnabled: attr('[data-room-environment-input-enabled]', 'data-room-environment-input-enabled'),
        frameInputReady: attr('[data-room-environment-frame-input-ready]', 'data-room-environment-frame-input-ready'),
        shieldCount: document.querySelectorAll('[data-room-environment-input-shield]').length,
        canvasCount: document.querySelectorAll('[data-view-display-canvas]').length,
        retryCount: document.querySelectorAll('[data-view-display-retry]').length,
        environmentText: text('[data-room-environment]'),
        displayMessage: text('.view-display-empty'),
        controlText: text('[data-room-environment-control-status]'),
      }
    })
    const screenshot = path.join(evidenceRoot, `MP-08-MP-10-${label}.png`)
    await page.screenshot({ path: screenshot, fullPage: false })
    return { ...projection, screenshot }
  }
  async function freshState(timeout = 40000) {
    const deadline = Date.now() + timeout
    let last
    while (Date.now() < deadline) {
      try { return await control('state') } catch (error) { last = error; await sleep(400) }
    }
    throw last ?? new Error('Room did not recover')
  }
  async function readyDisplay(timeout = 45000) {
    await page.locator(".view-connection-state[data-state='connected']").waitFor({ state: 'visible', timeout })
    await page.locator("[data-room-environment-status='ready']").waitFor({ state: 'visible', timeout })
  }
  const disposeObserver = client.onKernelEvent(() => {})
  async function observerReady() {
    const deadline = Date.now() + 40000
    while (Date.now() < deadline) {
      try {
        await client.connect()
        const response = await client.send(requests.attachToSessionRequest(ready.sessionId, `webfault-observer-${process.pid}`), { timeoutMs: 5000 })
        if (response?.SessionAttached) return
      } catch { /* The signed kernel must re-register before observation resumes. */ }
      await sleep(300)
    }
    throw new Error('observational relay client did not reattach')
  }
  async function requestAction(idempotencyKey) {
    try {
      await observerReady()
      const response = await client.send(requests.submitRoomEnvironmentActionRequest(
        ready.sessionId, latestEnvironment.runtime_generation, latestEnvironment.viewport.revision,
        idempotencyKey, { kind: 'keyboard_text', text: 'w'.repeat(140) }, 369,
      ), { timeoutMs: 45000 })
      return { response }
    } catch (error) { return { error: String(error.message).slice(0, 300) } }
  }
  async function waitNewAction(before, state, timeout = 15000) {
    const ids = new Set(before.page.actions.map(action => action.action_id))
    const deadline = Date.now() + timeout
    while (Date.now() < deadline) {
      const value = await control('state')
      const action = value.page.actions.find(item => !ids.has(item.action_id) && (!state || item.state === state))
      if (action) return action
      await sleep(100)
    }
    throw new Error(`operation boundary ${state ?? 'accepted'} was not observed`)
  }
  await page.evaluate(() => { window.__webfaultShell = document.querySelector('.terminal-app-workspace') })
  const original = await webSnapshot('initial')
  const selection = await readFile(path.join(coordinationDir, 'fault-selection.json'), 'utf8').then(JSON.parse).catch(error => {
    if (error.code === 'ENOENT') return null
    throw error
  })
  const supportedFaults = ['relay', 'display', 'controller', 'expired', 'stale-identity', 'queue', 'kernel']
  const supportedBoundaries = ['beginning', 'middle', 'commit']
  const faults = selection?.faults ?? supportedFaults
  const boundaries = selection?.boundaries ?? supportedBoundaries
  assert.ok(faults.length && faults.every(fault => supportedFaults.includes(fault)))
  assert.ok(boundaries.length && boundaries.every(boundary => supportedBoundaries.includes(boundary)))
  for (const fault of faults) for (const boundary of boundaries) {
    const label = `${fault}-${boundary}`
    const row = { fault, boundary, status: 'running', startedAt: new Date().toISOString(), assertions: [], errors: [] }
    rows.push(row); await report()
    let pending = null
    let queueBurst = null
    let faultApplied = false
    let faultSettled = false
    const check = (value, message) => { if (value) row.assertions.push(message); else row.errors.push(message) }
    try {
      await readyDisplay()
      const before = await freshState()
      latestEnvironment = before.environment
      row.before = { room: before.environment, completedActionIds: before.page.actions.filter(action => action.state === 'completed').map(action => action.action_id) }
      row.clientsBefore = await control('clients')
      // Every in-flight boundary is observed in the authoritative Action ledger.
      // Beginning is before submit, middle after running, commit after completed.
      if (boundary !== 'beginning') {
        pending = requestAction(`webfault-${attempt}-${label}`)
        row.operation = await waitNewAction(before, boundary === 'middle' ? 'running' : 'completed')
        if (boundary === 'commit') row.operationResponse = await pending
      }
      const triggerAt = Date.now()
      const identityBefore = identityFaultStats?.()
      if (fault === 'relay' || fault === 'expired' || fault === 'stale-identity') {
        if (fault !== 'relay') setIdentityMode(fault)
        row.trigger = await control('relay_stop')
      } else if (fault === 'kernel') row.trigger = await control('kernel_stop')
      else if (fault === 'display') row.trigger = await control('streamer_stop')
      else if (fault === 'controller') row.trigger = await control('controller_stop')
      else row.trigger = await control('relay_queue')
      faultApplied = true
      if (fault === 'queue') {
        await observerReady()
        row.queueBurstStartedAt = new Date().toISOString()
        queueBurst = (async () => {
          const values = []
          const deadline = Date.now() + 8000
          do {
            values.push(...await Promise.all(Array.from({ length: 128 }, () => client.send(requests.getRoomEnvironmentStateRequest(ready.sessionId), { timeoutMs: 8000 }).then(() => 'ok', error => /backpressure|queue.*full|no available capacity/i.test(error.message) ? 'backpressure' : 'other-error'))))
            await sleep(200)
          } while (Date.now() < deadline)
          row.queueBurstFinishedAt = new Date().toISOString()
          return values
        })()
      }
      if (fault === 'expired' || fault === 'stale-identity') await control('relay_start')
      // Observe degraded projection while the injected seam remains unhealthy.
      await sleep(fault === 'controller' ? 2000 : 5000)
      if (fault === 'expired' || fault === 'stale-identity') {
        const deadline = Date.now() + 15000
        while ((identityFaultStats?.()[fault] ?? 0) <= (identityBefore?.[fault] ?? 0) && Date.now() < deadline) await sleep(200)
        row.identityFault = identityFaultStats?.()
        check((row.identityFault?.[fault] ?? 0) > (identityBefore?.[fault] ?? 0), 'faulted identity was sent on a real Web relay handshake')
      }
      row.webFault = await webSnapshot(`${label}-fault`)
      row.faultSnapshotAt = new Date().toISOString()
      row.clientsFault = await control('clients')
      if (fault === 'controller') {
        row.controllerFaultState = await control('state')
        row.controllerFaultEvents = await control('controller_events')
      }
      row.detectionSampleMs = Date.now() - triggerAt
      check(row.webFault.timeOrigin === original.timeOrigin, 'document did not reload')
      check(row.webFault.shellMounted && row.webFault.shellSame, 'terminal shell remained mounted')
      if (['relay', 'kernel', 'display', 'expired', 'stale-identity'].includes(fault)) {
        if (fault === 'display') check(row.webFault.connection !== 'connected', 'display shows stream loss')
        else check(row.webFault.environmentStatus !== 'ready', 'Room projection marks home authority stale')
        check(row.webFault.frameInputReady !== 'true' || row.webFault.inputEnabled !== 'true', 'input is shielded while unhealthy')
      }
      if (fault === 'relay') check(row.clientsFault.local.daemonDisconnected === false, 'local TUI remains connected during relay loss')
      if (fault === 'kernel') check(row.clientsFault.local.daemonDisconnected === true, 'local TUI shows kernel disconnect')
      if (['relay', 'kernel'].includes(fault)) check(row.clientsFault.remote.daemonDisconnected === true, 'remote TUI shows disconnect')
      if (fault === 'queue') {
        const burst = await queueBurst
        row.queueResponses = Object.fromEntries(['ok', 'backpressure', 'other-error'].map(key => [key, burst.filter(value => value === key).length]))
        const healthUrl = new URL(ready.relayUrl); healthUrl.protocol = 'http:'; healthUrl.pathname = '/health'
        row.relayHealth = await fetch(healthUrl).then(value => value.json())
        check(row.queueResponses.backpressure > 0 || row.relayHealth.backpressure?.target_queue_full_count > 0 || row.relayHealth.backpressure?.slow_subscription_close_count > 0, 'queue saturation was measured, not assumed')
      }
      const recoveryAt = Date.now()
      if (fault === 'relay') await control('relay_start')
      else if (fault === 'kernel') await control('kernel_start')
      else if (fault === 'display') await control('streamer_start')
      else if (fault === 'expired' || fault === 'stale-identity' || fault === 'queue') {
        setIdentityMode('valid'); await control('relay_stop'); await control('relay_start')
      }
      faultSettled = true
      if (boundary !== 'beginning' && pending) row.operationResponse = await pending
      // Normal Web retry is allowed and recorded; it is not called automatic recovery.
      try { await readyDisplay(20000); row.recoveryPath = 'automatic' }
      catch {
        const retry = page.locator('[data-view-display-retry]')
        if (await retry.count()) { await retry.first().click(); row.recoveryPath = 'Web retry' }
        await readyDisplay(45000)
      }
      if (boundary === 'beginning') {
        latestEnvironment = (await freshState()).environment
        pending = requestAction(`webfault-${attempt}-${label}`)
        row.operation = await waitNewAction(before, 'completed')
        row.operationResponse = await pending
      }
      await control('room_status')
      const after = await freshState()
      const clientsDeadline = Date.now() + 15000
      do {
        row.clientsRecovery = await control('clients')
        if (!row.clientsRecovery.local.daemonDisconnected && !row.clientsRecovery.remote.daemonDisconnected) break
        await sleep(300)
      } while (Date.now() < clientsDeadline)
      row.webRecovery = await webSnapshot(`${label}-recovery`)
      row.recoveryMs = Date.now() - recoveryAt
      try { assertWebFaultRecovery(before, after); row.assertions.push('Room, Tabs and completed Action ledger preserved') }
      catch (error) { row.errors.push(error.message) }
      for (const kind of ['local', 'remote']) {
        check(row.clientsRecovery[kind].sessionId === ready.sessionId, `${kind} TUI retained Room`)
        check(row.clientsRecovery[kind].daemonDisconnected === false, `${kind} TUI recovered`)
      }
      if (row.operation) {
        const actions = after.page.actions.filter(item => item.action_id === row.operation.action_id)
        row.operationAfter = actions
        check(actions.length === 1 && !['running', 'queued'].includes(actions[0]?.state), 'in-flight operation settled once')
      }
      if (fault === 'controller') check(controllerFaultAttributed(row.controllerFaultState?.environment, row.controllerFaultEvents?.replay, row.operationAfter), 'controller fault is attributed in health or Action outcome')
      check(row.webRecovery.timeOrigin === original.timeOrigin && row.webRecovery.shellSame, 'recovery retained terminal shell')
      row.status = row.errors.length ? 'RED' : 'GREEN'
    } catch (error) {
      if (queueBurst) await queueBurst.catch(() => undefined)
      if (pending) row.operationResponse = await Promise.race([pending, sleep(1000).then(() => ({ pending: true }))])
      row.status = faultApplied ? 'RED' : 'BLOCKED'
      row.faultApplied = faultApplied
      row.firstFailingSeam = String(error.message).slice(0, 600)
      row.webFailure = await webSnapshot(`${label}-failure`).catch(() => null)
      // Always settle the owned fault before progressing or exiting.
      try {
        setIdentityMode('valid')
        if (faultApplied && !faultSettled) {
          if (fault === 'kernel') await control('kernel_start')
          else if (fault === 'display') await control('streamer_start')
          else if (['relay', 'expired', 'stale-identity', 'queue'].includes(fault)) { await control('relay_stop'); await control('relay_start') }
        }
      } catch (recoveryError) { row.cleanupFaultError = String(recoveryError.message).slice(0, 300) }
    }
    row.finishedAt = new Date().toISOString(); await report()
    console.log(`[MP-08 MP-10 webfault] ${label} ${row.status}`)
  }
  disposeObserver()
  return { reportPath, green: rows.filter(row => row.status === 'GREEN').length, red: rows.filter(row => row.status === 'RED').length, blocked: rows.filter(row => row.status === 'BLOCKED').length }
}
