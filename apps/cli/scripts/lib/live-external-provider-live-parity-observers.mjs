import { spawn } from "node:child_process"
import { rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { pathToFileURL } from "node:url"
import { setTimeout as sleep } from "node:timers/promises"

import {
  requiredAssistantMarkers,
  requiredToolMarkers,
  normalizeLifecycleStatus,
  unwrap,
} from "./live-external-provider-live-parity-common.mjs"
import { automationRequest, pipeChildLogs, waitForAutomation } from "./live-external-provider-live-parity-process.mjs"

import {
  getSessionHistoryBlobContentRequest,
  getSessionHistoryOutlineRequest,
  getSessionStateRequest,
} from "../../dist/ipc-requests.js"

const cliRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..", "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const cloudRepo = process.env.ARROBA_CLOUD_REPO ?? path.resolve(repoRoot, "..", "arroba-cloud")

export function startKernelMonitor({ client, sessionId, agentId, provider, marker, finalMarker, promptMarker, options }) {
  const samples = []
  let stopped = false
  const loop = (async () => {
    while (!stopped) {
      samples.push(await kernelSample({ client, sessionId, agentId, provider, marker, finalMarker, promptMarker }).catch((error) => ({
        at: new Date().toISOString(),
        error: String(error?.message ?? error),
      })))
      await sleep(options.pollMs)
    }
  })()
  return {
    async stop() {
      stopped = true
      await loop.catch(() => {})
      const finalSample = await kernelSample({ client, sessionId, agentId, provider, marker, finalMarker, promptMarker }).catch((error) => ({ error: String(error?.message ?? error) }))
      samples.push(finalSample)
      return summarizeSamples("kernel", samples, finalMarker)
    },
  }
}

export async function kernelSample({ client, sessionId, agentId, provider, marker, finalMarker, promptMarker }) {
  const stateResponse = await client.send(getSessionStateRequest(sessionId))
  const sessionState = stateResponse.SessionState ?? stateResponse.SessionStateLoaded ?? {}
  const session = sessionState.session
  const agent = (session?.agents ?? []).find((entry) => entry.id === agentId)
  const agentActivity = sessionState.agent_activity?.[agentId]
    ?? sessionState.agentActivity?.[agentId]
    ?? sessionState.agent_activity?.[String(agentId)]
    ?? null
  const outline = unwrap(await client.send(getSessionHistoryOutlineRequest(sessionId, [agentId], 1)), "SessionHistoryOutline")
  const text = await historyOutlineTextWithBlobContent({ client, sessionId, agentId, outline })
  return {
    at: new Date().toISOString(),
    surface: "kernel",
    status: agentStatus(agent, agentActivity),
    text,
    assistantMarkers: requiredAssistantMarkers.filter((entry) => text.includes(entry)),
    toolMarkers: requiredToolMarkers.filter((entry) => text.includes(entry)),
    finalSeen: text.includes(finalMarker),
    promptOccurrences: countPromptMarkerInHistoryOutline(outline, promptMarker),
    provider,
  }
}

export function countPromptMarkerInHistoryOutline(outline, promptMarker) {
  let count = 0
  for (const agent of outline.agents ?? []) {
    for (const turn of agent.turns ?? []) {
      if (turn.user_prompt?.entry?.text?.includes(promptMarker)) count += 1
    }
  }
  return count
}

export async function historyOutlineTextWithBlobContent({ client, sessionId, agentId, outline }) {
  const chunks = []
  for (const agent of outline.agents ?? []) {
    for (const turn of agent.turns ?? []) {
      if (turn.user_prompt?.entry?.text) chunks.push(turn.user_prompt.entry.text)
      const items = [
        ...(turn.entries ?? []).map((entry) => ({ sequence: entry.entry_index ?? 0, entry })),
        ...(turn.blobs ?? []).map((blob) => ({ sequence: blob.sequence_start ?? 0, blob })),
        ...(turn.summary ? [{ sequence: turn.summary.entry_index ?? Number.MAX_SAFE_INTEGER, entry: turn.summary }] : []),
      ].sort((left, right) => left.sequence - right.sequence)
      for (const item of items) {
        if ("entry" in item) {
          if (item.entry?.entry?.text) chunks.push(item.entry.entry.text)
          continue
        }
        const blob = item.blob
        if (!blob?.blob_id) {
          if (blob?.summary) chunks.push(blob.summary)
          continue
        }
        const blobText = await loadHistoryBlobText(client, sessionId, agent.agent_id ?? agentId, blob.blob_id)
          .catch(() => blob.summary ?? "")
        if (blobText) chunks.push(blobText)
      }
    }
  }
  return chunks.join("\n")
}

export async function loadHistoryBlobText(client, sessionId, agentId, blobId) {
  const response = unwrap(
    await client.send(getSessionHistoryBlobContentRequest(sessionId, agentId, blobId)),
    "SessionHistoryBlobContent",
  )
  return (response.entries ?? [])
    .map((entry) => entry.entry?.text ?? "")
    .join("\n")
}

export async function waitForKernelFinalIdle({ client, sessionId, agentId, provider, marker, finalMarker, promptMarker, options }) {
  const deadline = Date.now() + Math.min(120_000, Math.max(15_000, options.timeoutMs))
  let lastSample = null
  while (Date.now() < deadline) {
    lastSample = await kernelSample({ client, sessionId, agentId, provider, marker, finalMarker, promptMarker }).catch((error) => ({
      error: String(error?.message ?? error),
    }))
    if (!lastSample.error && lastSample.finalSeen && String(lastSample.status).toUpperCase() !== "WORKING") {
      await sleep(Math.max(750, options.pollMs))
      return lastSample
    }
    await sleep(options.pollMs)
  }
  throw new Error(`external turn did not settle to final idle in kernel history; last=${JSON.stringify(lastSample)}`)
}

export async function waitForSurfaceFinalIdle({ surface, sample, options }) {
  const deadline = Date.now() + Math.min(45_000, Math.max(10_000, options.timeoutMs))
  let lastSample = null
  let stableIdleCount = 0
  while (Date.now() < deadline) {
    lastSample = await sample().catch((error) => ({
      error: String(error?.message ?? error),
    }))
    const status = normalizeLifecycleStatus(lastSample.status)
    if (!lastSample.error && lastSample.finalSeen && status !== "WORKING") {
      stableIdleCount += 1
      if (stableIdleCount >= 2) {
        await sleep(Math.max(750, options.pollMs))
        return lastSample
      }
    } else {
      stableIdleCount = 0
    }
    await sleep(options.pollMs)
  }
  throw new Error(`${surface} did not observe final idle; last=${JSON.stringify(lastSample)}`)
}

export function agentStatus(agent, agentActivity = null) {
  if (agentActivity) {
    const status = String(agentActivity.status ?? "").toLowerCase()
    const promptStatus = String(agentActivity.prompt_status ?? agentActivity.promptStatus ?? "").toLowerCase()
    if (
      agentActivity.busy === true
      || agentActivity.active_turn
      || agentActivity.activeTurn
      || ["working", "running", "thinking", "streaming"].includes(status)
      || (promptStatus && promptStatus !== "none" && promptStatus !== "idle")
    ) {
      return "WORKING"
    }
    if (status === "error") return "ERROR"
    return "IDLE"
  }
  if (!agent) return "UNKNOWN"
  if (agent.is_processing || String(agent.state ?? "").toLowerCase() === "working") return "WORKING"
  return "IDLE"
}

export async function startTuiObserver({ sessionId, options, providerRoot }) {
  const socketPath = path.join(os.tmpdir(), `arroba-external-parity-${process.pid}-${sessionId}.sock`)
  const stdoutPath = path.join(providerRoot, "tui.stdout.log")
  const stderrPath = path.join(providerRoot, "tui.stderr.log")
  await rm(socketPath, { force: true }).catch(() => {})
  const child = spawn("bun", [
    path.join(cliRoot, "dist/index.js"),
    "--kernel-url",
    options.kernelUrl,
    "--session",
    sessionId,
    "--automation-socket",
    socketPath,
  ], {
    cwd: options.workspace,
    env: process.env,
    stdio: ["ignore", "pipe", "pipe"],
  })
  pipeChildLogs(child, stdoutPath, stderrPath)
  await waitForAutomation(socketPath, child)
  return { process: child, socketPath }
}

export function startTuiMonitor({ socketPath, provider, marker, finalMarker, promptMarker, options }) {
  const samples = []
  let stopped = false
  const loop = (async () => {
    while (!stopped) {
      samples.push(await tuiSample(socketPath, provider, marker, finalMarker, promptMarker).catch((error) => ({
        at: new Date().toISOString(),
        error: String(error?.message ?? error),
      })))
      await sleep(options.pollMs)
    }
  })()
  return {
    async stop() {
      stopped = true
      await loop.catch(() => {})
      samples.push(await tuiSample(socketPath, provider, marker, finalMarker, promptMarker).catch((error) => ({ error: String(error?.message ?? error) })))
      return summarizeSamples("tui", samples, finalMarker)
    },
  }
}

export async function tuiSample(socketPath, provider, marker, finalMarker, promptMarker) {
  const snapshot = await automationRequest(socketPath, { action: "snapshot" })
  const transcriptEntries = (snapshot.transcript?.entries ?? []).filter(Boolean)
  const paneEntries = Object.values(snapshot.agentPanes ?? {})
    .flat()
    .filter(Boolean)
  const transcriptText = transcriptEntries
    .map((entry) => entry.text ?? "")
    .join("\n")
  const paneText = paneEntries
    .map((entry) => entry.text ?? "")
    .join("\n")
  const text = [transcriptText, paneText].filter(Boolean).join("\n")
  const badge = snapshot.session?.agents?.[0]?.badge ?? null
  const entries = [...transcriptEntries, ...paneEntries]
  const promptPresent = entries.some((entry) =>
    String(entry.text ?? entry.entry?.text ?? "").includes(promptMarker),
  ) || text.includes(promptMarker)
  return {
    at: new Date().toISOString(),
    surface: "tui",
    status: badge?.label ?? badge?.tone ?? "UNKNOWN",
    text,
    assistantMarkers: requiredAssistantMarkers.filter((entry) => text.includes(entry)),
    toolMarkers: requiredToolMarkers.filter((entry) => text.includes(entry)),
    finalSeen: text.includes(finalMarker),
    promptOccurrences: promptPresent ? 1 : 0,
    collapsedEntries: entries.filter((entry) => entry.blobCollapsed === true).length,
    expandedEntries: entries.filter((entry) => entry.blobCollapsed === false).length,
    provider,
  }
}

export async function startWebObserver({ sessionId, webUrl, providerRoot }) {
  const { launchChromiumBrowser } = await import(pathToFileURL(path.join(cloudRepo, "scripts/lib/playwright.mjs")).href)
  const browser = await launchChromiumBrowser({ headless: process.env.ARROBA_EXTERNAL_PARITY_WEB_HEADFUL !== "1" })
  const context = await browser.newContext({ baseURL: webUrl, viewport: { width: 1500, height: 980 } })
  const page = await context.newPage()
  page.on("pageerror", (error) => console.log(`[web-pageerror] ${error.message}`))
  await page.goto("/waiting-room", { waitUntil: "domcontentloaded" })
  await waitForProductKernelReady(page, 90_000)
  await waitForWaitingRoomSessionRow(page, sessionId, 90_000)
  await waitForWaitingRoomSessionRowEnabled(page, sessionId, 30_000)
  await captureWebScreenshot(page, path.join(providerRoot, "web-waiting-room.png"))
  await openSessionFromWaitingRoom(page, sessionId)
  await page.locator("[data-freeform-pane-grid], .freeform-workspace").first().waitFor({ timeout: 90_000 })
  await clearSessionPickerOverlay(page)
  await captureWebScreenshot(page, path.join(providerRoot, "web-opened.png"))
  return { browser, context, page }
}

export async function waitForProductKernelReady(page, timeoutMs) {
  await waitForWebCondition(page, async () => {
    const textFor = (selector) => document.querySelector(selector)?.textContent?.trim() ?? ""
    const status = textFor("[data-waiting-room-footer-status]")
    const kernel = textFor("[data-waiting-room-footer-kernel]")
    const banner = textFor("[data-waiting-room-status]")
    return status === "ready"
      && Boolean(kernel)
      && !/no kernel connected|loading/i.test(kernel)
      && /Kernel waiting room ready/i.test(banner)
  }, timeoutMs, "product waiting room did not reach connected kernel state")
}

export async function waitForWaitingRoomSessionRow(page, sessionId, timeoutMs) {
  await page.locator(`[data-waiting-session-id="${cssAttributeValue(sessionId)}"]`).first().waitFor({ timeout: timeoutMs })
}

export async function waitForWaitingRoomSessionRowEnabled(page, sessionId, timeoutMs) {
  await waitForWebCondition(page, (targetSessionId) => {
    const row = document.querySelector(`[data-waiting-session-id="${targetSessionId}"]`)
    return Boolean(row)
      && !row.hasAttribute("disabled")
      && row.getAttribute("aria-disabled") !== "true"
      && !row.classList.contains("disabled")
  }, timeoutMs, `waiting-room session row ${sessionId} did not become enabled`, sessionId)
}

export async function openSessionFromWaitingRoom(page, sessionId) {
  const joinRow = page.locator("[data-waiting-row-key='join']").first()
  await joinRow.click()
  const pickerRow = page.locator(`[data-session-picker-session-id="${cssAttributeValue(sessionId)}"]`).first()
  await pickerRow.waitFor({ timeout: 30_000 })
  await pickerRow.evaluate((element) => {
    if (element instanceof HTMLElement) element.click()
  })
}

export async function clearSessionPickerOverlay(page) {
  const overlay = page.locator("[data-session-picker-close]").first()
  if (!(await overlay.isVisible().catch(() => false))) return
  await page.mouse.click(40, 40)
  await sleep(500)
  if (!(await overlay.isVisible().catch(() => false))) return
  await page.reload({ waitUntil: "domcontentloaded" })
  await page.locator("[data-freeform-pane-grid], .freeform-workspace").first().waitFor({ timeout: 90_000 })
}

export async function expandWebTranscript(page, options) {
  const deadline = Date.now() + Math.min(30_000, Math.max(5_000, options.timeoutMs))
  while (Date.now() < deadline) {
    const clicked = await page.evaluate(() => {
      let count = 0
      for (const button of document.querySelectorAll('[data-freeform-turn-toggle][aria-expanded="false"]')) {
        if (button instanceof HTMLElement) {
          button.click()
          count += 1
        }
      }
      for (const button of document.querySelectorAll('.freeform-blob-header[aria-expanded="false"]')) {
        if (button instanceof HTMLElement) {
          button.click()
          count += 1
        }
      }
      return count
    }).catch(() => 0)
    if (!clicked) break
    await sleep(350)
  }
  await sleep(1_000)
}

export async function expandTuiTranscriptBlobs(socketPath, options) {
  const deadline = Date.now() + Math.min(60_000, Math.max(5_000, options.timeoutMs))
  const attempts = new Map()
  while (Date.now() < deadline) {
    const snapshot = await automationRequest(socketPath, { action: "snapshot" }).catch(() => null)
    const entries = tuiSnapshotEntries(snapshot)
    const turnTargets = entries.filter(({ entry }) => {
      const turnId = Number(entry?.turnId)
      const entryId = Number(entry?.id)
      return Number.isInteger(turnId)
        && Number.isInteger(entryId)
        && entry?.role === "turn_toggle"
        && entry?.toggleMode === "expand"
        && (attempts.get(`turn:${entry.agentId ?? "primary"}:${turnId}:${entryId}`) ?? 0) < 3
    }).slice(0, 16)
    if (turnTargets.length > 0) {
      for (const target of turnTargets) {
        const turnId = Number(target.entry.turnId)
        const entryId = Number(target.entry.id)
        const key = `turn:${target.agentId ?? "primary"}:${turnId}:${entryId}`
        attempts.set(key, (attempts.get(key) ?? 0) + 1)
        await automationRequest(socketPath, {
          action: "toggle_turn",
          agentId: target.agentId,
          turnId,
          entryId,
        }).catch(() => null)
      }
      await sleep(750)
      continue
    }

    const blobTargets = entries.filter(({ entry }) => {
      const entryId = Number(entry?.id)
      const key = `blob:${entry.agentId ?? "primary"}:${String(entry?.historyBlobId ?? entryId)}`
      return Number.isInteger(entryId)
        && entry?.blobCollapsible === true
        && entry?.blobCollapsed !== false
        && (attempts.get(key) ?? 0) < 4
    }).slice(0, 24)
    if (blobTargets.length === 0) break
    for (const target of blobTargets) {
      const entryId = Number(target.entry.id)
      const key = `blob:${target.agentId ?? "primary"}:${String(target.entry.historyBlobId ?? entryId)}`
      attempts.set(key, (attempts.get(key) ?? 0) + 1)
      await automationRequest(socketPath, {
        action: "toggle_blob",
        agentId: target.agentId,
        entryId,
        collapsed: false,
      }).catch(() => null)
    }
    await sleep(750)
  }
  await sleep(1_000)
}

export function tuiSnapshotEntries(snapshot) {
  if (!snapshot) return []
  const transcriptEntries = (snapshot.transcript?.entries ?? [])
    .filter(Boolean)
    .map((entry) => ({ entry, agentId: null }))
  const paneEntries = Object.entries(snapshot.agentPanes ?? {}).flatMap(([agentId, entries]) =>
    (entries ?? []).filter(Boolean).map((entry) => ({ entry, agentId })),
  )
  return [...transcriptEntries, ...paneEntries]
}

export async function waitForWebCondition(page, predicate, timeoutMs, message, arg = undefined) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      if (await page.evaluate(predicate, arg)) return
    } catch (error) {
      lastError = error
    }
    await sleep(250)
  }
  throw new Error(`${message}${lastError ? `: ${lastError.message}` : ""}`)
}

export function cssAttributeValue(value) {
  return String(value).replace(/\\/g, "\\\\").replace(/"/g, '\\"')
}

export function startWebMonitor({ page, provider, marker, finalMarker, promptMarker, providerRoot, options }) {
  const samples = []
  let stopped = false
  const loop = (async () => {
    while (!stopped) {
      samples.push(await webSample(page, provider, marker, finalMarker, promptMarker).catch((error) => ({
        at: new Date().toISOString(),
        error: String(error?.message ?? error),
      })))
      await sleep(options.pollMs)
    }
  })()
  return {
    async stop() {
      stopped = true
      await loop.catch(() => {})
      samples.push(await webSample(page, provider, marker, finalMarker, promptMarker).catch((error) => ({ error: String(error?.message ?? error) })))
      await captureWebScreenshot(page, path.join(providerRoot, "web-final.png"))
      return summarizeSamples("web", samples, finalMarker)
    },
  }
}

export async function captureWebScreenshot(page, file) {
  await page.screenshot({
    path: file,
    fullPage: false,
    timeout: 10_000,
  }).catch((error) => {
    void writeFile(`${file}.error.txt`, String(error?.stack ?? error), "utf8").catch(() => {})
  })
}

export async function webSample(page, provider, marker, finalMarker, promptMarker) {
  return await page.evaluate(({ provider, marker, promptMarker, requiredAssistantMarkers, requiredToolMarkers, finalMarker }) => {
    const output = document.querySelector("[data-terminal-output]") ?? document.body
    const text = output.textContent ?? ""
    const scrollElement = output instanceof HTMLElement ? output : document.scrollingElement
    const bottomDistance = scrollElement
      ? Math.max(0, scrollElement.scrollHeight - scrollElement.clientHeight - scrollElement.scrollTop)
      : null
    const badges = [...document.querySelectorAll(".freeform-status-badge")].map((element) => element.textContent?.trim() ?? "")
    const turnButtons = [...document.querySelectorAll("[data-freeform-turn-toggle]")].map((element) => element.getAttribute("aria-expanded"))
    const blobButtons = [...document.querySelectorAll(".freeform-blob-header")].map((element) => element.getAttribute("aria-expanded"))
    const promptOccurrences = [...document.querySelectorAll(".freeform-user-prompt")]
      .filter((element) => (element.textContent ?? "").includes(promptMarker))
      .length
    return {
      at: new Date().toISOString(),
      surface: "web",
      status: badges.includes("WORKING") ? "WORKING" : badges.includes("IDLE") ? "IDLE" : "UNKNOWN",
      text,
      assistantMarkers: requiredAssistantMarkers.filter((entry) => text.includes(entry)),
      toolMarkers: requiredToolMarkers.filter((entry) => text.includes(entry)),
      finalSeen: text.includes(finalMarker),
      promptOccurrences: promptOccurrences || text.split(promptMarker).length - 1,
      bottomDistance,
      turnExpandedCount: turnButtons.filter((value) => value === "true").length,
      turnCollapsedCount: turnButtons.filter((value) => value === "false").length,
      blobExpandedCount: blobButtons.filter((value) => value === "true").length,
      blobCollapsedCount: blobButtons.filter((value) => value === "false").length,
      provider,
    }
  }, { provider, marker, promptMarker, requiredAssistantMarkers, requiredToolMarkers, finalMarker })
}

export function summarizeSamples(surface, samples, finalMarker) {
  const valid = samples.filter((sample) => !sample.error)
  const text = valid.map((sample) => sample.text ?? "").join("\n")
  const firstFinalSampleIndex = valid.findIndex((sample) => sample.finalSeen)
  const preFinalSamples = firstFinalSampleIndex >= 0 ? valid.slice(0, firstFinalSampleIndex) : valid
  const finalAndLaterSamples = firstFinalSampleIndex >= 0 ? valid.slice(firstFinalSampleIndex) : []
  const countMax = (entries, key) => Math.max(0, ...entries.map((sample) => Number(sample[key] ?? 0)).filter(Number.isFinite))
  return {
    surface,
    sampleCount: samples.length,
    errorCount: samples.length - valid.length,
    samples,
    assistantMarkersSeen: requiredAssistantMarkers.filter((marker) => text.includes(marker)),
    toolMarkersSeen: requiredToolMarkers.filter((marker) => text.includes(marker)),
    finalSeen: text.includes(finalMarker),
    statuses: valid.map((sample) => sample.status).filter(Boolean),
    maxBottomDistance: Math.max(0, ...valid.map((sample) => Number(sample.bottomDistance ?? 0)).filter(Number.isFinite)),
    preFinalMaxBottomDistance: Math.max(0, ...preFinalSamples.map((sample) => Number(sample.bottomDistance ?? 0)).filter(Number.isFinite)),
    promptOccurrenceMax: Math.max(0, ...valid.map((sample) => Number(sample.promptOccurrences ?? 0)).filter(Number.isFinite)),
    firstFinalSampleIndex,
    preFinalSampleCount: preFinalSamples.length,
    preFinalMaxAssistantMarkers: Math.max(0, ...preFinalSamples.map((sample) => sample.assistantMarkers?.length ?? 0)),
    preFinalMaxToolMarkers: Math.max(0, ...preFinalSamples.map((sample) => sample.toolMarkers?.length ?? 0)),
    preFinalStatuses: preFinalSamples.map((sample) => sample.status).filter(Boolean),
    preFinalMaxTurnCollapsedCount: countMax(preFinalSamples, "turnCollapsedCount"),
    preFinalMaxTurnExpandedCount: countMax(preFinalSamples, "turnExpandedCount"),
    finalMaxTurnCollapsedCount: countMax(finalAndLaterSamples, "turnCollapsedCount"),
    finalMaxTurnExpandedCount: countMax(finalAndLaterSamples, "turnExpandedCount"),
    finalMaxBlobCollapsedCount: countMax(finalAndLaterSamples, "blobCollapsedCount"),
  }
}
