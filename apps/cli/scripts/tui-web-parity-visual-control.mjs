#!/usr/bin/env node
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import net from 'node:net'
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { automationNoticeTexts } from './lib/room-tui-notices.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const defaultLatestManifest = path.join(repoRoot, 'target', 'live-tui-web-parity-visual-session', 'latest.json')
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function assertSameHostRelayUrl(value) {
  const endpoint = new URL(value)
  assert.ok(['ws:', 'wss:'].includes(endpoint.protocol), 'remote TUI relay URL is invalid')
  const host = endpoint.hostname.toLowerCase().replace(/^\[|\]$/g, '')
  const isLoopbackIpv4 = /^127(?:\.\d{1,3}){3}$/.test(host)
    && host.split('.').every((part) => Number(part) <= 255)
  assert.ok(host === 'localhost' || host === '::1' || isLoopbackIpv4,
    'Drill C requires a same-host loopback relay')
  return endpoint
}

export async function captureDrillCRoomCheckpoint({ client, requests, sessionId, sliceId }) {
  assert.ok(sessionId && sliceId, 'Drill C checkpoint requires the real Room session and slice ids')
  const environment = unwrap(await client.send(requests.getRoomEnvironmentStateRequest(sessionId)), 'RoomEnvironmentState').environment
  assert.equal(environment.session_id, sessionId, 'kernel Room Environment session mismatch')
  assert.equal(environment.lifecycle, 'ready', 'Drill C Room Environment must be ready')
  assert.ok(environment.environment_id, 'kernel Room Environment omitted environment id')
  assert.ok(environment.focused_tab_id, 'kernel Room Environment has no focused browser tab')

  const sliceBinding = unwrap(await client.send(requests.getRoomEnvironmentSliceRequest(sessionId)), 'RoomEnvironmentSlice').binding
  assert.ok(sliceBinding, 'kernel Room Environment has no slice binding')
  assert.equal(sliceBinding.session_id, sessionId, 'slice binding Room mismatch')
  assert.equal(sliceBinding.slice_id, sliceId, 'slice binding id mismatch')

  const resourceInventory = unwrap(await client.send(
    requests.getRoomEnvironmentResourceInventoryRequest(sessionId, sliceId),
  ), 'RoomEnvironmentResourceInventory').inventory
  assert.equal(resourceInventory.session_id, sessionId, 'Room resource inventory Room mismatch')
  assert.equal(resourceInventory.environment_id, environment.environment_id, 'Room resource inventory Environment mismatch')
  assert.equal(resourceInventory.slice_id, sliceId, 'Room resource inventory slice mismatch')
  assert.ok(resourceInventory.browser_ids.length > 0, 'Room resource inventory omitted browser identity')
  assert.ok(resourceInventory.profile_ids.length > 0, 'Room resource inventory omitted browser profile identity')

  const actionPage = unwrap(await client.send(
    requests.listRoomEnvironmentActionHistoryRequest(sessionId, null, 100),
  ), 'RoomEnvironmentActionHistoryListed').page
  assert.ok(Array.isArray(actionPage.actions), 'kernel Room Action history response is malformed')
  return {
    capturedAt: new Date().toISOString(),
    sessionId,
    environment,
    sliceBinding,
    resourceInventory,
    actions: actionPage.actions,
    highestActionSequence: actionPage.actions.reduce(
      (highest, action) => Math.max(highest, action.sequence),
      0,
    ),
  }
}

export function assertDrillCSharedRoomEvidence({ baseline, checkpoint, web, tui, remoteTui }) {
  assert.ok(baseline && checkpoint && web && tui, 'Drill C evidence requires baseline, kernel, Web, and local TUI observations')
  assert.ok(remoteTui, 'Drill C evidence requires a remote TUI observation through the relay')
  assert.equal(baseline.sessionId, baseline.environment?.session_id, 'baseline Room identity mismatch')
  assert.equal(checkpoint.environment?.session_id, baseline.sessionId, 'TUI observer saw a different Room')
  assert.equal(checkpoint.environment?.environment_id, baseline.environment.environment_id, 'Room Environment changed after Computer work')
  assert.equal(checkpoint.environment?.runtime_generation, baseline.environment.runtime_generation, 'Room runtime changed after Computer work')
  assert.deepEqual(checkpoint.sliceBinding, baseline.sliceBinding, 'Room slice binding changed after Computer work')
  assert.deepEqual(displayIdentity(checkpoint), displayIdentity(baseline), 'canonical display identity changed after Computer work')
  assert.equal(checkpoint.environment.focused_tab_id, baseline.environment.focused_tab_id, 'focused tab changed after Computer work')

  const focusedTab = checkpoint.environment.tabs.find((tab) => tab.tab_id === checkpoint.environment.focused_tab_id)
  assert.ok(focusedTab, 'kernel Room Environment omitted the focused tab record')
  assert.equal(web.schema, 'chariox.browser_computer.drill_c.web_observer.v1', 'unsupported Web observer evidence schema')
  assert.equal(web.status, 'observed', 'Web observer did not report an observed Room')
  assert.equal(web.observer, 'web', 'Drill C evidence did not come from the Web observer')
  assert.equal(web.client, 'production-local-web-view', 'Drill C requires evidence from the production Web client')
  const webObservedAt = Date.parse(web.observedAt)
  assert.ok(Number.isFinite(webObservedAt), 'Web observation timestamp is invalid')
  assert.ok(webObservedAt >= Date.parse(baseline.capturedAt), 'Web observation predates the Room baseline')
  assert.ok(webObservedAt <= Date.parse(checkpoint.capturedAt), 'Web observation postdates the TUI/kernel checkpoint')
  assert.equal(web.sessionId, baseline.sessionId, 'Web observer saw a different Room')
  assert.equal(web.environmentId, checkpoint.environment.environment_id, 'Web observer saw a different Room Environment')
  assert.equal(web.sliceId, checkpoint.sliceBinding.slice_id, 'Web observer saw a different slice')
  assert.equal(web.runtimeGeneration, checkpoint.environment.runtime_generation, 'Web observer saw a different Room runtime')
  assert.deepEqual(normalizeWebDisplay(web.display), displayIdentity(checkpoint), 'Web observer saw a different canonical display')
  assert.equal(web.browser?.focusedTabId, focusedTab.tab_id, 'Web observer saw a different focused tab')
  assert.deepEqual(web.browser?.tab, {
    tabId: focusedTab.tab_id,
    url: focusedTab.url,
    title: focusedTab.title,
    documentRevision: focusedTab.document_revision,
  }, 'Web browser state differs from the kernel Room Environment')

  const actors = new Map(checkpoint.environment.actors.map((actor) => [actor.actor_id, actor]))
  const newComputerActions = checkpoint.actions.filter((action) =>
    action.sequence > baseline.highestActionSequence
    && action.mode === 'computer'
    && action.state === 'completed'
    && actors.get(action.actor_id)?.kind === 'agent',
  ).sort((left, right) => left.sequence - right.sequence)
  assert.ok(newComputerActions.length > 0, 'kernel history contains no completed agent Computer work after the baseline')
  const computerAction = newComputerActions.at(-1)
  assert.deepEqual(web.actions?.computer, actionIdentity(computerAction), 'Web observer did not observe the kernel Computer Action')

  const takeoverAction = checkpoint.actions.find((action) => action.action_id === web.actions?.webTakeover?.actionId)
  assert.ok(takeoverAction, 'Web takeover Action is absent from kernel-owned Action history')
  assert.equal(actors.get(takeoverAction.actor_id)?.kind, 'human', 'Web takeover Action is not attributed to a human actor')
  assert.deepEqual(web.actions.webTakeover, actionIdentity(takeoverAction), 'Web observer Action identity differs from kernel history')
  assert.ok(takeoverAction.sequence > computerAction.sequence, 'Web takeover must follow Computer work in kernel Action history')
  assert.ok(webObservedAt >= takeoverAction.submitted_at_ms, 'Web observation predates its takeover Action')

  assert.equal(tui.sessionId, baseline.sessionId, 'TUI observer attached to a different Room')
  assert.equal(remoteTui.sessionId, baseline.sessionId, 'remote TUI observer attached to a different Room')
  assert.ok(tui.attachmentId && remoteTui.attachmentId && tui.attachmentId !== remoteTui.attachmentId,
    'Drill C requires distinct TUI attachments')
  assert.equal(remoteTui.transport?.kind, 'relay', 'remote TUI observer did not use the relay')
  assert.equal(remoteTui.transport.targetDaemonId, baseline.sliceBinding.owner_kernel_id,
    'remote TUI observer targeted a different home kernel')
  assertSameHostRelayUrl(remoteTui.transport.relayUrl)
  assertTuiRoomStatus(tui.statusNotice, checkpoint.environment, focusedTab)
  assertTuiActionVisible(tui.actionsNotice, computerAction)
  assertTuiActionVisible(tui.actionsNotice, takeoverAction)
  assertTuiRoomStatus(remoteTui.statusNotice, checkpoint.environment, focusedTab)
  assertTuiActionVisible(remoteTui.actionsNotice, computerAction)
  assertTuiActionVisible(remoteTui.actionsNotice, takeoverAction)

  return {
    schema: 'chariox.browser_computer.drill_c.live_evidence.v2',
    status: 'passed',
    observedAt: new Date().toISOString(),
    sameRoom: true,
    sameEnvironment: true,
    sameSlice: true,
    sameTab: true,
    sameDisplay: true,
    room: baseline.sessionId,
    environmentId: checkpoint.environment.environment_id,
    sliceId: checkpoint.sliceBinding.slice_id,
    runtimeGeneration: checkpoint.environment.runtime_generation,
    display: displayIdentity(checkpoint),
    browser: {
      focusedTabId: focusedTab.tab_id,
      tab: web.browser.tab,
    },
    actions: {
      computer: actionIdentity(computerAction),
      webTakeover: actionIdentity(takeoverAction),
    },
    observers: {
      localTui: { attachmentId: tui.attachmentId, statusNotice: tui.statusNotice, actionsNotice: tui.actionsNotice },
      remoteTui: {
        attachmentId: remoteTui.attachmentId,
        transport: remoteTui.transport,
        statusNotice: remoteTui.statusNotice,
        actionsNotice: remoteTui.actionsNotice,
      },
      web: { client: web.client, observedAt: web.observedAt },
    },
  }
}

function displayIdentity(checkpoint) {
  const environment = checkpoint.environment
  const inventory = checkpoint.resourceInventory
  return {
    environmentId: environment.environment_id,
    sliceId: checkpoint.sliceBinding.slice_id,
    runtimeGeneration: environment.runtime_generation,
    viewport: environment.viewport,
    browserIds: [...inventory.browser_ids].sort(),
    profileIds: [...inventory.profile_ids].sort(),
  }
}

function actionIdentity(action) {
  return {
    actionId: action.action_id,
    actorId: action.actor_id,
    sequence: action.sequence,
    mode: action.mode,
    kind: action.kind,
    state: action.state,
  }
}

function normalizeWebDisplay(display) {
  assert.ok(display && typeof display === 'object', 'Web observer omitted canonical display identity')
  assert.ok(Array.isArray(display.browserIds), 'Web observer omitted browser identities')
  assert.ok(Array.isArray(display.profileIds), 'Web observer omitted browser profile identities')
  return {
    ...display,
    browserIds: [...display.browserIds].sort(),
    profileIds: [...display.profileIds].sort(),
  }
}

function assertTuiRoomStatus(statusNotice, environment, focusedTab) {
  assert.equal(typeof statusNotice, 'string', 'TUI observer omitted /room status')
  const lines = statusNotice.split('\n')
  assert.equal(lines[0], `Room environment ${environment.environment_id}`, 'TUI observer saw a different Room Environment')
  const viewport = environment.viewport
  assert.ok(lines.includes(
    `viewport=${viewport.desktop_pixel_width}x${viewport.desktop_pixel_height} css=${viewport.css_width}x${viewport.css_height} scale=${viewport.device_scale_factor} revision=${viewport.revision}`,
  ), 'TUI observer did not show the canonical display viewport')
  assert.ok(lines.some((line) => line.startsWith(`tab=${focusedTab.tab_id} `)), 'TUI observer did not show the kernel focused tab')
}

function assertTuiActionVisible(actionsNotice, action) {
  assert.equal(typeof actionsNotice, 'string', 'TUI observer omitted /room actions')
  const line = actionsNotice.split('\n').find((candidate) => candidate.startsWith(`#${action.sequence} ${action.action_id} `))
  assert.ok(line, `TUI observer did not show kernel Action ${action.action_id}`)
  assert.ok(line.includes(`actor=${action.actor_id} ${action.mode}:${action.kind} `), 'TUI action actor or kind differs from kernel history')
  assert.ok(line.endsWith(`state=${action.state} submitted_at_ms=${action.submitted_at_ms}`), 'TUI action outcome differs from kernel history')
}

function unwrap(response, key) {
  if (response && typeof response === 'object' && key in response) return response[key]
  throw new Error(`kernel response omitted ${key}`)
}

function parseArgs(argv) {
  const options = {
    manifestPath: defaultLatestManifest,
    action: 'snapshot',
    label: null,
    json: null,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    const next = () => {
      const value = argv[index + 1]
      if (!value) throw new Error(`missing value for ${arg}`)
      index += 1
      return value
    }
    if (arg === '--manifest') options.manifestPath = path.resolve(next())
    else if (arg === '--action') options.action = next()
    else if (arg === '--label') options.label = next()
    else if (arg === '--json') options.json = JSON.parse(next())
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node apps/cli/scripts/tui-web-parity-visual-control.mjs [--manifest PATH] [--action snapshot|assert|assert-blobs|capture-layouts|waiting-room|steer-queued|cancel-queued|toggle-first-blob|expand-history-blob|toggle-first-turn|send|drill-c-verify|report] [--label LABEL] [--json JSON]')
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function createAutomationClient(socketPath) {
  const socket = net.createConnection(socketPath)
  socket.setEncoding('utf8')
  let nextId = 1
  let buffer = ''
  const pending = new Map()
  socket.on('data', (chunk) => {
    buffer += chunk
    while (buffer.includes('\n')) {
      const newline = buffer.indexOf('\n')
      const line = buffer.slice(0, newline).trim()
      buffer = buffer.slice(newline + 1)
      if (!line) continue
      const response = JSON.parse(line)
      const deferred = pending.get(response.id)
      if (!deferred) continue
      pending.delete(response.id)
      if (response.ok) deferred.resolve(response.data)
      else deferred.reject(new Error(response.error ?? 'automation command failed'))
    }
  })
  socket.on('error', (error) => {
    for (const deferred of pending.values()) deferred.reject(error)
    pending.clear()
  })
  return {
    send(action, fields = {}) {
      const id = nextId++
      const request = { id, action, ...fields }
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject })
        socket.write(`${JSON.stringify(request)}\n`)
      })
    },
    close() {
      socket.destroy()
    },
  }
}

function paneEntries(snapshot, manifest) {
  const panes = snapshot.agentPanes && typeof snapshot.agentPanes === 'object' ? snapshot.agentPanes : {}
  const focused = manifest.agentId ?? snapshot.session?.focusedAgentId
  const entries = focused && Array.isArray(panes[focused]) ? panes[focused] : []
  const source = entries.length > 0 ? entries : Array.isArray(snapshot.transcript?.entries) ? snapshot.transcript.entries : []
  return source.filter((entry) => entry && typeof entry === 'object')
}

function firstCollapsedBlob(snapshot, manifest) {
  const entries = paneEntries(snapshot, manifest)
  return entries.find((entry) =>
    entry
    && entry.blobCollapsible === true
    && entry.blobCollapsed === true
    && !entry.historyBlobId,
  ) ?? entries.find((entry) =>
    entry
    && entry.blobCollapsible === true
    && (entry.blobCollapsed === true || entry.historyBlobId),
  )
}

function firstVisibleTurn(snapshot, manifest) {
  return paneEntries(snapshot, manifest).find((entry) => Number.isInteger(entry?.turnId))
}

function firstHistoryBlob(snapshot, manifest) {
  return paneEntries(snapshot, manifest).find((entry) =>
    entry
    && entry.blobCollapsible === true
    && entry.blobCollapsed === true
    && entry.historyBlobId
    && entry.historyBlobLoaded !== true,
  )
}

function queuedEntries(snapshot, manifest) {
  return paneEntries(snapshot, manifest).filter((entry) => entry?.queuedPrompt)
}

function queuedPromptStrips(snapshot) {
  const strips = snapshot.queuedPromptStrips && typeof snapshot.queuedPromptStrips === 'object'
    ? snapshot.queuedPromptStrips
    : {}
  return Object.fromEntries(
    Object.entries(strips).map(([agentId, strip]) => [
      agentId,
      {
        selectedIndex: Number.isInteger(strip?.selectedIndex) ? strip.selectedIndex : 0,
        items: Array.isArray(strip?.items) ? strip.items : [],
      },
    ]),
  )
}

function truncateText(value, width) {
  const text = String(value ?? '').replace(/\s+/g, ' ').trim()
  if (text.length <= width) return text
  return `${text.slice(0, Math.max(0, width - 1))}…`
}

function line(label, value, width) {
  const prefix = label ? `${label}: ` : ''
  return truncateText(`${prefix}${value ?? ''}`, width)
}

function summarizeSnapshot(snapshot, manifest) {
  const entries = paneEntries(snapshot, manifest)
  const queued = queuedEntries(snapshot, manifest)
  const assistantEntries = entries.filter((entry) => entry.role === 'assistant' && !entry.hidden)
  const userEntries = entries.filter((entry) => entry.role === 'user' && !entry.hidden)
  const errorEntries = entries.filter((entry) => entry.role === 'error' && !entry.hidden)
  const collapsedBlobs = entries.filter((entry) => entry.blobCollapsible === true && entry.blobCollapsed === true)
  const expandedBlobs = entries.filter((entry) => entry.blobCollapsible === true && entry.blobCollapsed === false)
  const historyPlaceholders = entries.filter((entry) => entry.historyBlobId && !entry.historyBlobLoaded)
  const loadedHistoryBlobs = entries.filter((entry) => entry.historyBlobLoaded === true && entry.historyBlobSourceId)
  const hiddenTurnEntries = entries.filter((entry) => entry.hidden)
  const collapsedBlobRoles = collapsedBlobs.map((entry) => entry.role)
  const expandedBlobRoles = expandedBlobs.map((entry) => entry.role)
  const loadedHistoryBlobRoles = loadedHistoryBlobs.map((entry) => entry.role)
  const loadedHistoryBlobTexts = loadedHistoryBlobs.map((entry) => entry.text).filter(Boolean)
  return {
    screen: snapshot.screen,
    statusLine: snapshot.statusLine,
    session: snapshot.session,
    footer: snapshot.footer,
    entryCount: entries.length,
    assistantEntryCount: assistantEntries.length,
    userEntryCount: userEntries.length,
    errorEntryCount: errorEntries.length,
    collapsedBlobCount: collapsedBlobs.length,
    expandedBlobCount: expandedBlobs.length,
    collapsedBlobRoles,
    expandedBlobRoles,
    historyPlaceholderCount: historyPlaceholders.length,
    loadedHistoryBlobCount: loadedHistoryBlobs.length,
    loadedHistoryBlobRoles,
    loadedHistoryBlobTexts,
    hiddenTurnEntryCount: hiddenTurnEntries.length,
    visibleTexts: entries.map((entry) => entry.text).filter(Boolean),
    queuedPrompts: queued.map((entry) => entry.queuedPrompt),
    queuedPromptStrips: queuedPromptStrips(snapshot),
    waitingRoomRows: snapshot.waitingRoom?.rows ?? null,
  }
}

function terminalCaptureLines(snapshot, manifest, summary, width) {
  const contentWidth = Math.max(40, width)
  if (snapshot.waitingRoom) {
    const rows = Array.isArray(snapshot.waitingRoom.rows) ? snapshot.waitingRoom.rows : []
    return [
      line('screen', snapshot.screen ?? 'waiting-room', contentWidth),
      line('status', snapshot.statusLine ?? '', contentWidth),
      line('footer', snapshot.footer?.summary ?? snapshot.footer ?? '', contentWidth),
      ...rows.slice(0, 12).map((row) => {
        const marker = row.focused ? '>' : ' '
        return truncateText(`${marker} ${row.title ?? ''} ${row.value ?? ''}`.trimEnd(), contentWidth)
      }),
    ].filter(Boolean)
  }

  const strips = queuedPromptStrips(snapshot)
  const focusedStrip = manifest.agentId ? strips[manifest.agentId] : Object.values(strips)[0]
  const entries = paneEntries(snapshot, manifest)
  const roleCounts = summary.collapsedBlobRoles.reduce((counts, role) => {
    counts[role] = (counts[role] ?? 0) + 1
    return counts
  }, {})
  const roleSummary = Object.entries(roleCounts).map(([role, count]) => `${role}:${count}`).join(' ')
  const loadedHistoryLines = (summary.loadedHistoryBlobTexts ?? []).slice(0, 3).map((text, index) => {
    const role = summary.loadedHistoryBlobRoles?.[index] ?? 'entry'
    return line('loaded history', `${role} ${text}`, contentWidth)
  })
  const queueLines = (focusedStrip?.items ?? []).slice(0, 4).map((item, index) => {
    const selected = index === focusedStrip.selectedIndex ? '>' : ' '
    const actions = `${item.canSteer ? '[S]' : '[S disabled]'} ${item.canCancel ? '[C]' : '[C disabled]'}`
    return truncateText(`${selected} ${item.status} ${item.attachmentCount ? `${item.attachmentCount} file ` : ''}${item.prompt} ${actions}`, contentWidth)
  })
  const transcriptLines = entries
    .filter((entry) => !entry.hidden)
    .slice(-10)
    .map((entry) => {
      const marker = entry.blobCollapsible
        ? entry.blobCollapsed ? '▸' : '▾'
        : ' '
      return truncateText(`${marker} ${entry.role ?? 'entry'} ${entry.text ?? ''}`, contentWidth)
    })
  return [
    line('screen', snapshot.screen ?? 'attached', contentWidth),
    line('status', snapshot.statusLine ?? '', contentWidth),
    line('footer', snapshot.footer?.summary ?? snapshot.footer ?? '', contentWidth),
    line('queue', focusedStrip ? `${focusedStrip.items.length} prompts selected=${focusedStrip.selectedIndex}` : 'none', contentWidth),
    ...queueLines,
    line('blobs', roleSummary || 'none', contentWidth),
    ...loadedHistoryLines,
    ...transcriptLines,
  ].filter(Boolean)
}

async function waitForLoadedHistoryBlob(automation, manifest, placeholder) {
  const deadline = Date.now() + 10_000
  let snapshot = await automation.send('snapshot')
  while (Date.now() < deadline) {
    const entries = paneEntries(snapshot, manifest)
    const placeholderStillVisible = entries.some((entry) => entry.historyBlobId === placeholder.historyBlobId)
    const loadedEntries = entries.filter((entry) =>
      entry.historyBlobLoaded === true
      && entry.historyBlobSourceId === placeholder.historyBlobId
      && entry.historyBlobSourceAgentId === placeholder.historyBlobAgentId,
    )
    if (!placeholderStillVisible && loadedEntries.length > 0 && loadedEntries.some((entry) => entry.text)) {
      return snapshot
    }
    await sleep(100)
    snapshot = await automation.send('snapshot')
  }
  throw new Error(`timed out waiting for loaded history blob ${placeholder.historyBlobId}`)
}

async function run(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', reject)
    child.on('close', (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

async function renderTerminalScreenshot(outputDir, fileName, title, lines, options = {}) {
  await mkdir(outputDir, { recursive: true })
  const columns = options.columns ?? 80
  const pixelWidth = Math.max(860, 16 * columns + 96)
  const height = Math.max(300, 112 + lines.length * 24)
  const svgPath = path.join(outputDir, `${fileName}.svg`)
  const pngPath = path.join(outputDir, fileName)
  const escaped = (value) => String(value)
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
  const body = lines.map((captureLine, index) => (
    `<text x="48" y="${112 + index * 24}" fill="#d9e2ec" font-size="18">${escaped(captureLine)}</text>`
  )).join('\n')
  await writeFile(svgPath, `<svg xmlns="http://www.w3.org/2000/svg" width="${pixelWidth}" height="${height}">
<rect width="100%" height="100%" fill="#101820"/>
<rect x="28" y="28" width="${pixelWidth - 56}" height="${height - 56}" rx="8" fill="#141f2b" stroke="#3b5269"/>
<text x="48" y="72" fill="#ffffff" font-family="Menlo, Consolas, monospace" font-size="22" font-weight="700">${escaped(title)}</text>
<g font-family="Menlo, Consolas, monospace">${body}</g>
</svg>`, 'utf8')
  const result = await run('sips', ['-s', 'format', 'png', svgPath, '--out', pngPath])
  if (result.code !== 0) {
    throw new Error(`failed to render terminal screenshot ${fileName}: ${result.stdout}\n${result.stderr}`)
  }
  await rm(svgPath, { force: true })
  return pngPath
}

async function writeTerminalCaptures(manifest, label, snapshot, summary) {
  const outputDir = path.join(manifest.evidenceDir, 'terminal-captures')
  const captures = []
  for (const spec of [
    { id: '80x24', columns: 80 },
    { id: '120x32', columns: 120 },
  ]) {
    const lines = terminalCaptureLines(snapshot, manifest, summary, spec.columns)
    assert(
      lines.every((captureLine) => captureLine.length <= spec.columns),
      `${label} ${spec.id} capture line exceeded ${spec.columns} columns`,
    )
    const base = `${label}.${spec.id}`
    const textPath = path.join(outputDir, `${base}.txt`)
    await mkdir(outputDir, { recursive: true })
    await writeFile(textPath, `${lines.join('\n')}\n`, 'utf8')
    const screenshotPath = await renderTerminalScreenshot(
      outputDir,
      `${base}.png`,
      `TUI/Web Parity ${label} ${spec.id}`,
      lines,
      spec,
    )
    captures.push({ ...spec, textPath, screenshotPath })
  }
  return captures
}

function assertBlobSnapshot(snapshot, manifest, summary) {
  const entries = paneEntries(snapshot, manifest)
  const collapsedBlobs = entries.filter((entry) => entry.blobCollapsible === true && entry.blobCollapsed === true)
  const expandedBlobs = entries.filter((entry) => entry.blobCollapsible === true && entry.blobCollapsed === false)
  const expandedNormalRoles = entries
    .filter((entry) => ['assistant', 'user', 'error'].includes(entry.role) && !entry.hidden)
    .map((entry) => entry.role)
  for (const role of ['reasoning', 'tool']) {
    assert(
      collapsedBlobs.some((entry) => entry.role === role),
      `expected a collapsed ${role} blob in the live agent pane`,
    )
  }
  assert(
    collapsedBlobs.some((entry) => entry.role === 'status' || entry.role === 'notice'),
    'expected a collapsed status/notice runtime blob in the live agent pane',
  )
  assert(
    collapsedBlobs.some((entry) => entry.historyBlobId && entry.historyBlobLoaded === false),
    'expected at least one collapsed lazy history blob placeholder',
  )
  assert(
    expandedNormalRoles.includes('assistant') && expandedNormalRoles.includes('user') && expandedNormalRoles.includes('error'),
    `expected assistant/user/error entries to stay expanded, saw ${expandedNormalRoles.join(',')}`,
  )
  assert.equal(
    expandedBlobs.some((entry) => ['assistant', 'user', 'error'].includes(entry.role)),
    false,
    'assistant/user/error entries should not be represented as collapsed/expanded blob rows',
  )
  assert(
    summary.collapsedBlobCount >= 3,
    `expected at least three collapsed blobs, saw ${summary.collapsedBlobCount}`,
  )
}

function assertWaitingRoomSnapshot(snapshot, manifest) {
  assert(snapshot.waitingRoom, 'snapshot should be detached in waiting room')
  const rows = Array.isArray(snapshot.waitingRoom.rows) ? snapshot.waitingRoom.rows : []
  assert(rows.length > 0, 'waiting room should render at least one row')
  const rowIds = rows.map((row) => row.id).filter(Boolean)
  const idleRowId = `session:${manifest.waitingRoom?.idleSessionId}`
  const doneRowId = `session:${manifest.waitingRoom?.doneSessionId}`
  assert(
    rowIds.includes(idleRowId),
    `waiting room rows should include seeded idle session ${idleRowId}`,
  )
  assert(
    rowIds.includes(doneRowId),
    `waiting room rows should include seeded done session ${doneRowId}`,
  )
  const rowText = rows.map((row) => `${row.focused ? '> ' : '  '}${row.title ?? ''} ${row.value ?? ''}`.trimEnd())
  assert(
    rowText.some((line) => line.includes('wr-idle-parity')),
    'waiting room rows should include idle session alias text',
  )
  assert(
    rowText.some((line) => line.includes('wr-done-parity') || line.includes('DONE')),
    'waiting room rows should include done session state',
  )
  const sessionRowText = rows
    .filter((row) => typeof row.id === 'string' && row.id.startsWith('session:'))
    .map((row) => `${row.title ?? ''} ${row.value ?? ''}`.trimEnd())
  assert(
    sessionRowText.every((line) => line.length <= 120),
    `waiting room session row projection should stay bounded for visual drill summaries: ${sessionRowText.find((line) => line.length > 120)}`,
  )
}

function assertParitySnapshot(snapshot, manifest, summary) {
  if (snapshot.waitingRoom) {
    assert(Array.isArray(snapshot.waitingRoom.rows), 'waiting room snapshot should expose rows')
    assert(snapshot.waitingRoom.rows.length > 0, 'waiting room should render at least one row')
    return
  }

  const strips = queuedPromptStrips(snapshot)
  const focusedStrip = manifest.agentId ? strips[manifest.agentId] : Object.values(strips)[0]
  assert(focusedStrip, 'focused agent should expose a queued prompt strip projection')
  assert(focusedStrip.items.length >= 2, `expected at least two queued prompt strip items, saw ${focusedStrip.items.length}`)
  assert(Number.isInteger(focusedStrip.selectedIndex), 'queued prompt strip should expose selectedIndex')
  assert(
    focusedStrip.items.some((item) => item.canSteer === true),
    'queued prompt strip should expose a steerable item',
  )
  assert(
    focusedStrip.items.some((item) => item.canCancel === true),
    'queued prompt strip should expose a cancellable item',
  )
  assert.equal(
    summary.queuedPrompts.length,
    0,
    'queued prompts should not be duplicated as transcript rows when the strip projection is available',
  )
  assert(
    summary.collapsedBlobCount >= 3,
    `expected collapsed reasoning/tool/status blobs, saw ${summary.collapsedBlobCount}`,
  )
}

async function writeEvidence(manifest, label, snapshot, summary) {
  await mkdir(manifest.evidenceDir, { recursive: true })
  const safe = label.replace(/[^a-zA-Z0-9_.-]+/g, '-')
  const snapshotPath = path.join(manifest.evidenceDir, `${safe}.snapshot.json`)
  const summaryPath = path.join(manifest.evidenceDir, `${safe}.summary.json`)
  await writeFile(snapshotPath, `${JSON.stringify(snapshot, null, 2)}\n`, 'utf8')
  await writeFile(summaryPath, `${JSON.stringify(summary, null, 2)}\n`, 'utf8')
  return { snapshotPath, summaryPath }
}

async function writeReport(manifest) {
  const report = {
    schema: 'chariox.tui_web_parity_visual_validation_report.v1',
    completedAt: new Date().toISOString(),
    manifestPath: manifest.manifestPath,
    rootDir: manifest.rootDir,
    evidenceDir: manifest.evidenceDir,
    sessionId: manifest.sessionId,
    agentId: manifest.agentId,
    drillCSharedBrowserComputer: manifest.schema === 'chariox.drill_c.live_observer_session.v1'
      ? {
        status: 'not_proven',
        reason: 'run drill-c-verify after Computer work and Web takeover to compare live observers',
      }
      : {
        status: 'not_proven',
        reason: 'this TUI terminal visual session does not create a Room Environment or observe Web takeover',
      },
    requirements: {
      visibleTuiSession: 'validated by VS Code screen captures plus automation snapshots from this session',
      agentPaneBlobs: 'requires initial, assert-blobs, and blob-expanded snapshots',
      loadedHistoryBlobDetail: 'requires expand-history-blob and loaded-history-layout snapshots proving lazy history placeholders become loaded text entries',
      queuedPromptSteeringCancel: 'requires attached-initial and post-raw-steer snapshots plus narrow/wide terminal captures after raw PTY shortcut delivery',
      footersAndPromptArea: 'requires generated narrow/wide terminal screenshots and status/footer snapshot summaries',
      waitingRoom: 'requires request-waiting-room snapshot with seeded idle/done rows and bounded session row summaries',
      queuedPromptStripProjection: 'automation snapshots expose queuedPromptStrips with selectedIndex, prompt text, status, attachment count, steer, and cancel actionability',
      queuedPromptActions: 'steer-queued and cancel-queued route through the same queued_prompt_action automation path as the TUI strip action handler',
    },
  }
  await mkdir(path.dirname(manifest.reportPath), { recursive: true })
  await writeFile(manifest.reportPath, `${JSON.stringify(report, null, 2)}\n`, 'utf8')
  return report
}

async function observeRoomFromTui(automation) {
  const attachmentSnapshot = await automation.send('snapshot')
  const statusSnapshot = await automation.send('submit_prompt', { prompt: '/room status' })
  const statusNotice = automationNoticeTexts(statusSnapshot)
    .findLast((notice) => notice.startsWith('Room environment '))
  const actionsSnapshot = await automation.send('submit_prompt', { prompt: '/room actions 100' })
  const actionsNotice = automationNoticeTexts(actionsSnapshot)
    .findLast((notice) => notice.startsWith('Room actions'))
  assert.ok(statusNotice, 'TUI observer did not return a /room status notice')
  assert.ok(actionsNotice, 'TUI observer did not return a /room actions notice')
  return {
    attachmentId: attachmentSnapshot.attachmentId ?? null,
    sessionId: attachmentSnapshot.session?.id ?? null,
    statusNotice,
    actionsNotice,
  }
}

async function verifyDrillC(manifest, automation) {
  assertDrillCLiveObserverManifest(manifest)

  const { LocalIpcClient } = await import(pathToFileURL(path.join(repoRoot, 'packages/kernel-client/dist/ipc.js')).href)
  const requests = await import(pathToFileURL(path.join(repoRoot, 'packages/kernel-client/dist/ipc-requests.js')).href)
  const client = new LocalIpcClient(manifest.kernelUrl)
  const remoteAutomation = createAutomationClient(manifest.remoteAutomationSocket)
  try {
    const checkpoint = await captureDrillCRoomCheckpoint({
      client,
      requests,
      sessionId: manifest.sessionId,
      sliceId: manifest.sliceId,
    })
    const web = JSON.parse(await readFile(manifest.webObservationPath, 'utf8'))
    const tui = await observeRoomFromTui(automation)
    const remoteTui = {
      ...await observeRoomFromTui(remoteAutomation),
      transport: {
        kind: 'relay',
        relayUrl: manifest.remoteRelay.url,
        targetDaemonId: manifest.remoteRelay.targetDaemonId,
      },
    }
    const report = assertDrillCSharedRoomEvidence({ baseline: manifest.baseline, checkpoint, web, tui, remoteTui })
    const reportPath = path.join(manifest.evidenceDir, 'drill-c-live-observation.json')
    await mkdir(path.dirname(reportPath), { recursive: true })
    await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, 'utf8')
    return { report, reportPath }
  } finally {
    await client.close?.().catch(() => {})
    remoteAutomation.close()
  }
}

export function assertDrillCLiveObserverManifest(manifest) {
  assert.equal(
    manifest?.schema,
    'chariox.drill_c.live_observer_session.v1',
    'Drill C verification requires a live Room observer manifest; dev-stub visual sessions cannot pass',
  )
  assert.ok(manifest.baseline && manifest.sessionId && manifest.sliceId, 'live observer manifest is incomplete')
  const endpoint = new URL(manifest.kernelUrl)
  assert.ok(['ws:', 'wss:'].includes(endpoint.protocol), 'live observer manifest kernel URL is invalid')
  assert.ok(Number.isSafeInteger(manifest.baseline.highestActionSequence), 'live observer baseline omitted kernel Action sequence')
  assert.equal(manifest.baseline.sessionId, manifest.sessionId, 'live observer baseline Room mismatch')
  assert.equal(manifest.baseline.environment?.session_id, manifest.sessionId, 'live observer baseline Environment mismatch')
  assert.ok(manifest.baseline.environment?.environment_id, 'live observer baseline omitted Environment identity')
  assert.equal(manifest.baseline.sliceBinding?.slice_id, manifest.sliceId, 'live observer baseline slice mismatch')
  assert.equal(manifest.baseline.resourceInventory?.environment_id, manifest.baseline.environment.environment_id, 'live observer baseline inventory mismatch')
  assert.equal(manifest.baseline.resourceInventory?.slice_id, manifest.sliceId, 'live observer baseline inventory slice mismatch')
  assert.ok(path.isAbsolute(manifest.webObservationPath), 'Web observer report path must be absolute')
  assert.ok(typeof manifest.remoteAutomationSocket === 'string' && path.isAbsolute(manifest.remoteAutomationSocket),
    'remote TUI automation socket path must be absolute')
  assert.ok(typeof manifest.automationSocket === 'string' && path.isAbsolute(manifest.automationSocket),
    'local TUI automation socket path must be absolute')
  assert.notEqual(manifest.remoteAutomationSocket, manifest.automationSocket,
    'local and remote TUI automation sockets must be distinct')
  assert.equal(manifest.remoteRelay?.targetDaemonId, manifest.baseline.sliceBinding?.owner_kernel_id,
    'remote TUI relay must target the home kernel')
  assert.ok(typeof manifest.remoteRelay?.url === 'string', 'remote TUI relay URL is missing')
  const relayEndpoint = assertSameHostRelayUrl(manifest.remoteRelay.url)
  assert.notEqual(relayEndpoint.origin, endpoint.origin, 'remote TUI relay cannot be the direct kernel endpoint')
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const manifest = JSON.parse(await readFile(options.manifestPath, 'utf8'))
  manifest.manifestPath = options.manifestPath
  const automation = createAutomationClient(manifest.automationSocket)
  try {
    await automation.send('ping')
    let snapshot
    if (options.action === 'snapshot') {
      snapshot = await automation.send('snapshot')
    } else if (options.action === 'drill-c-verify') {
      const verified = await verifyDrillC(manifest, automation)
      console.log(JSON.stringify(verified, null, 2))
      return
    } else if (options.action === 'assert') {
      snapshot = await automation.send('snapshot')
    } else if (options.action === 'assert-blobs') {
      snapshot = await automation.send('snapshot')
    } else if (options.action === 'capture-layouts') {
      snapshot = await automation.send('snapshot')
    } else if (options.action === 'waiting-room') {
      snapshot = await automation.send('request_waiting_room')
    } else if (options.action === 'steer-queued' || options.action === 'cancel-queued') {
      snapshot = await automation.send('queued_prompt_action', {
        queuedPromptAction: options.action === 'steer-queued' ? 'steer' : 'cancel',
        ...(options.json && typeof options.json === 'object' ? options.json : {}),
      })
    } else if (options.action === 'toggle-first-blob') {
      const before = await automation.send('snapshot')
      const entry = firstCollapsedBlob(before, manifest)
      assert(entry, 'no collapsed blob or lazy history blob found')
      snapshot = await automation.send('toggle_blob', {
        agentId: manifest.agentId,
        entryId: entry.id,
        collapsed: false,
      })
      const toggled = paneEntries(snapshot, manifest).find((candidate) => candidate.id === entry.id)
      assert(
        toggled?.blobCollapsed === false || toggled?.historyBlobLoading === true || toggled?.historyBlobLoaded === true,
        'toggled blob should be expanded or show lazy history loading in the resulting snapshot',
      )
    } else if (options.action === 'expand-history-blob') {
      const before = await automation.send('snapshot')
      const entry = firstHistoryBlob(before, manifest)
      assert(entry, 'no collapsed lazy history blob found')
      await automation.send('toggle_blob', {
        agentId: manifest.agentId,
        entryId: entry.id,
        collapsed: false,
      })
      snapshot = await waitForLoadedHistoryBlob(automation, manifest, entry)
    } else if (options.action === 'toggle-first-turn') {
      const before = await automation.send('snapshot')
      const entry = firstVisibleTurn(before, manifest)
      assert(entry, 'no visible turn entry found')
      snapshot = await automation.send('toggle_turn', {
        agentId: manifest.agentId,
        turnId: entry.turnId,
        entryId: entry.id,
      })
    } else if (options.action === 'send') {
      const request = options.json
      assert(request && typeof request === 'object', '--json is required for --action send')
      snapshot = await automation.send(request.action, request)
    } else if (options.action === 'report') {
      const report = await writeReport(manifest)
      console.log(JSON.stringify(report, null, 2))
      return
    } else {
      throw new Error(`unknown action: ${options.action}`)
    }
    const label = options.label ?? options.action
    const summary = summarizeSnapshot(snapshot, manifest)
    if (options.action === 'assert') {
      assertParitySnapshot(snapshot, manifest, summary)
    } else if (options.action === 'assert-blobs') {
      assertBlobSnapshot(snapshot, manifest, summary)
    } else if (options.action === 'waiting-room') {
      assertWaitingRoomSnapshot(snapshot, manifest)
    }
    const evidence = await writeEvidence(manifest, label, snapshot, summary)
    const captures = options.action === 'capture-layouts'
      ? await writeTerminalCaptures(manifest, label, snapshot, summary)
      : []
    console.log(JSON.stringify({ label, evidence, captures, summary }, null, 2))
  } finally {
    automation.close()
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`[tui-web-parity-visual-control] ${error.stack ?? error.message}`)
    process.exitCode = 1
  })
}
