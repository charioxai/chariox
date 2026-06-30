#!/usr/bin/env node
import assert from 'node:assert/strict'
import net from 'node:net'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'

function parseArgs(argv) {
  const options = {
    manifestPath: path.resolve('target/live-tui-web-parity-visual-session/latest.json'),
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
      console.log('Usage: node apps/cli/scripts/tui-web-parity-visual-control.mjs [--manifest PATH] [--action snapshot|assert|toggle-first-blob|toggle-first-turn|send|report] [--label LABEL] [--json JSON]')
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
  return paneEntries(snapshot, manifest).find((entry) =>
    entry
    && entry.blobCollapsible === true
    && (entry.blobCollapsed === true || entry.historyBlobId),
  )
}

function firstVisibleTurn(snapshot, manifest) {
  return paneEntries(snapshot, manifest).find((entry) => Number.isInteger(entry?.turnId))
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

function summarizeSnapshot(snapshot, manifest) {
  const entries = paneEntries(snapshot, manifest)
  const queued = queuedEntries(snapshot, manifest)
  const assistantEntries = entries.filter((entry) => entry.role === 'assistant' && !entry.hidden)
  const userEntries = entries.filter((entry) => entry.role === 'user' && !entry.hidden)
  const errorEntries = entries.filter((entry) => entry.role === 'error' && !entry.hidden)
  const collapsedBlobs = entries.filter((entry) => entry.blobCollapsible === true && entry.blobCollapsed === true)
  const expandedBlobs = entries.filter((entry) => entry.blobCollapsible === true && entry.blobCollapsed === false)
  const historyPlaceholders = entries.filter((entry) => entry.historyBlobId && !entry.historyBlobLoaded)
  const hiddenTurnEntries = entries.filter((entry) => entry.hidden)
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
    historyPlaceholderCount: historyPlaceholders.length,
    hiddenTurnEntryCount: hiddenTurnEntries.length,
    visibleTexts: entries.map((entry) => entry.text).filter(Boolean),
    queuedPrompts: queued.map((entry) => entry.queuedPrompt),
    queuedPromptStrips: queuedPromptStrips(snapshot),
    waitingRoomRows: snapshot.waitingRoom?.rows ?? null,
  }
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
    schema: 'arroba.tui_web_parity_visual_validation_report.v1',
    completedAt: new Date().toISOString(),
    manifestPath: manifest.manifestPath,
    rootDir: manifest.rootDir,
    evidenceDir: manifest.evidenceDir,
    sessionId: manifest.sessionId,
    agentId: manifest.agentId,
    requirements: {
      visibleTuiSession: 'validated by VS Code screen captures plus automation snapshots from this session',
      agentPaneBlobs: 'requires initial, blob-expanded, and turn-toggle snapshots',
      queuedPromptSteeringCancel: 'requires queued-initial, post-keyboard-steer, and post-cancel snapshots',
      footersAndPromptArea: 'requires screen captures and status/footer snapshot summaries',
      waitingRoom: 'requires waiting-room snapshot and screen capture',
      queuedPromptStripProjection: 'automation snapshots expose queuedPromptStrips with selectedIndex, prompt text, status, attachment count, steer, and cancel actionability',
    },
  }
  await mkdir(path.dirname(manifest.reportPath), { recursive: true })
  await writeFile(manifest.reportPath, `${JSON.stringify(report, null, 2)}\n`, 'utf8')
  return report
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
    } else if (options.action === 'assert') {
      snapshot = await automation.send('snapshot')
    } else if (options.action === 'toggle-first-blob') {
      const before = await automation.send('snapshot')
      const entry = firstCollapsedBlob(before, manifest)
      assert(entry, 'no collapsed blob or lazy history blob found')
      snapshot = await automation.send('toggle_blob', {
        agentId: manifest.agentId,
        entryId: entry.id,
        collapsed: false,
      })
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
    }
    const evidence = await writeEvidence(manifest, label, snapshot, summary)
    console.log(JSON.stringify({ label, evidence, summary }, null, 2))
  } finally {
    automation.close()
  }
}

main().catch((error) => {
  console.error(`[tui-web-parity-visual-control] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
