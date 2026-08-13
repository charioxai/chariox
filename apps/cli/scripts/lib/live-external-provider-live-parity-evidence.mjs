import { spawn } from "node:child_process"
import { readFileSync } from "node:fs"
import { readFile, readdir, stat, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"

import {
  assertion,
  countOccurrences,
  dedupe,
  fail,
  markdownCell,
  normalizeLifecycleStatus,
  requiredAssistantMarkers,
  requiredToolMarkers,
} from "./live-external-provider-live-parity-common.mjs"

export async function snapshotProviderTranscript({ provider, providerSessionId, providerRoot, finalMarker, promptMarker }) {
  if (!providerSessionId) {
    return { surface: "provider", found: false, reason: "provider session id unavailable" }
  }
  if (provider === "opencode") {
    const sqliteSnapshot = await snapshotOpenCodeSqliteTranscript({ providerSessionId, providerRoot, finalMarker, promptMarker })
    if (sqliteSnapshot.found) return sqliteSnapshot
  }
  const path = await findProviderTranscriptPath(provider, providerSessionId)
  if (!path) {
    return {
      surface: "provider",
      found: false,
      providerSessionId,
      reason: `no ${provider} transcript path matched provider session ${providerSessionId}`,
    }
  }
  const text = await readFile(path, "utf8")
  const artifactPath = pathJoin(providerRoot, `provider-transcript${pathExt(path)}`)
  await writeFile(artifactPath, text, "utf8")
  const semanticEvidence = semanticJsonlTranscriptEvidence(provider, text, { finalMarker, promptMarker })
  return {
    surface: "provider",
    found: true,
    providerSessionId,
    path,
    artifactPath,
    byteLength: Buffer.byteLength(text),
    ...(semanticEvidence
      ? { semanticEvidence: true, ...semanticEvidence }
      : {
          assistantMarkersSeen: requiredAssistantMarkers.filter((marker) => text.includes(marker)),
          toolMarkersSeen: requiredToolMarkers.filter((marker) => text.includes(marker)),
          finalSeen: text.includes(finalMarker),
          promptOccurrences: countOccurrences(text, promptMarker),
        }),
  }
}

export async function snapshotOpenCodeSqliteTranscript({ providerSessionId, providerRoot, finalMarker, promptMarker }) {
  for (const root of providerTranscriptRoots("opencode")) {
    const dbPath = path.join(root, "opencode.db")
    if (!(await stat(dbPath).catch(() => null))) continue
    const sessionId = sqliteString(providerSessionId)
    const query = `
      select 'message' as kind, id, session_id, null as message_id, time_created, time_updated, data
        from message
       where session_id = '${sessionId}'
      union all
      select 'part' as kind, id, session_id, message_id, time_created, time_updated, data
        from part
       where session_id = '${sessionId}'
       order by time_created, id
    `
    const capture = await captureCommand("sqlite3", ["-json", dbPath, query], { maxBytes: 16 * 1024 * 1024 }).catch((error) => ({
      ok: false,
      stderr: String(error?.message ?? error),
    }))
    if (!capture.ok || !capture.stdout.trim()) continue
    const rows = parseJson(capture.stdout)
    if (!Array.isArray(rows) || rows.length === 0) continue
    const text = JSON.stringify(rows, null, 2)
    const transcriptEvidence = classifyOpenCodeSqliteTranscriptRows(rows, { finalMarker, promptMarker })
    const artifactPath = pathJoin(providerRoot, "provider-transcript.sqlite.json")
    await writeFile(artifactPath, text, "utf8")
    return {
      surface: "provider",
      found: true,
      providerSessionId,
      path: dbPath,
      artifactPath,
      byteLength: Buffer.byteLength(text),
      rowCount: rows.length,
      semanticEvidence: true,
      ...transcriptEvidence,
    }
  }
  return {
    surface: "provider",
    found: false,
    providerSessionId,
    reason: `no OpenCode SQLite transcript rows matched provider session ${providerSessionId}`,
  }
}

export function classifyCodexJsonlTranscriptValues(values, { finalMarker, promptMarker } = {}) {
  const assistantText = []
  const userText = []
  const toolEvents = []
  const reasoningText = []
  for (const value of values ?? []) {
    const payload = value?.payload ?? value
    if (value?.type === "response_item") {
      const itemType = payload?.type
      if (itemType === "message") {
        const texts = codexContentTexts(payload.content)
        if (payload.role === "assistant") assistantText.push(...texts)
        if (payload.role === "user") userText.push(...texts)
      } else if (itemType === "agentMessage") {
        assistantText.push(...codexContentTexts(payload.text ?? payload.content))
      } else if (itemType === "reasoning") {
        reasoningText.push(...codexContentTexts(payload.summary ?? payload.content ?? payload.text))
      } else if (codexToolItemType(itemType)) {
        toolEvents.push(JSON.stringify(payload))
      }
      continue
    }
    if (value?.type !== "event_msg") continue
    if (payload?.type === "agent_message" && typeof payload.message === "string") {
      assistantText.push(payload.message)
    } else if (payload?.type === "user_message" && typeof payload.message === "string") {
      userText.push(payload.message)
    } else if (payload?.type === "agent_reasoning" && typeof payload.text === "string") {
      reasoningText.push(payload.text)
    } else if (payload?.type === "mcp_tool_call_end") {
      toolEvents.push(JSON.stringify(payload))
    }
  }
  return classifiedTranscriptEvidence({
    assistantText: dedupe(assistantText),
    userText: dedupe(userText),
    toolEvents,
    reasoningText: dedupe(reasoningText),
    finalMarker,
    promptMarker,
  })
}

export function classifyClaudeJsonlTranscriptValues(values, { finalMarker, promptMarker } = {}) {
  const assistantText = []
  const userText = []
  const toolEvents = []
  const reasoningText = []
  for (const value of values ?? []) {
    const recordType = value?.type
    const message = value?.message ?? value
    if (claudeInternalResumeEntry(value, message)) continue
    const content = message?.content ?? value?.content
    if (recordType === "assistant") {
      for (const block of claudeContentBlocks(content)) {
        const blockType = block?.type ?? "text"
        if (blockType === "text") {
          assistantText.push(...claudeBlockTexts(block, ["text", "content"]))
        } else if (blockType === "thinking") {
          reasoningText.push(...claudeBlockTexts(block, ["thinking", "text", "content"]))
        } else if (blockType === "tool_use" || blockType === "tool_result") {
          toolEvents.push(JSON.stringify(block))
        }
      }
    } else if (recordType === "user") {
      for (const block of claudeContentBlocks(content)) {
        const blockType = block?.type ?? "text"
        if (blockType === "tool_result") {
          toolEvents.push(JSON.stringify(block))
        } else if (blockType === "text") {
          userText.push(...claudeBlockTexts(block, ["text", "content"]))
        }
      }
    }
  }
  return classifiedTranscriptEvidence({
    assistantText,
    userText,
    toolEvents,
    reasoningText,
    finalMarker,
    promptMarker,
  })
}

function semanticJsonlTranscriptEvidence(provider, text, markers) {
  const values = jsonlValues(text)
  if (provider === "codex") return classifyCodexJsonlTranscriptValues(values, markers)
  if (provider === "claude") return classifyClaudeJsonlTranscriptValues(values, markers)
  return null
}

function codexContentTexts(content) {
  if (typeof content === "string") return [content]
  if (!Array.isArray(content)) return []
  return content.flatMap((block) => {
    if (typeof block === "string") return [block]
    if (typeof block?.text === "string") return [block.text]
    if (typeof block?.content === "string") return [block.content]
    return []
  })
}

function codexToolItemType(itemType) {
  return [
    "function_call",
    "function_call_output",
    "custom_tool_call",
    "custom_tool_call_output",
    "commandExecution",
    "fileChange",
    "mcpToolCall",
    "dynamicToolCall",
    "collabAgentToolCall",
    "local_shell_call",
  ].includes(itemType)
}

function claudeContentBlocks(content) {
  if (Array.isArray(content)) return content
  if (typeof content === "string") return [{ type: "text", text: content }]
  return content && typeof content === "object" ? [content] : []
}

function claudeBlockTexts(block, keys) {
  for (const key of keys) {
    if (typeof block?.[key] === "string") return [block[key]]
  }
  return []
}

function claudeInternalResumeEntry(value, message) {
  return (value?.type === "user" && value?.isMeta === true)
    || (value?.type === "assistant"
      && message?.model === "<synthetic>"
      && value?.isApiErrorMessage !== true)
}

function classifiedTranscriptEvidence({ assistantText, userText, toolEvents, reasoningText, finalMarker, promptMarker }) {
  return {
    assistantMarkersSeen: markersInTexts(requiredAssistantMarkers, assistantText),
    toolMarkersSeen: markersInTexts(requiredToolMarkers, toolEvents),
    reasoningMarkersSeen: markersInTexts(requiredAssistantMarkers, reasoningText),
    finalSeen: Boolean(finalMarker) && assistantText.some((text) => text.includes(finalMarker)),
    promptOccurrences: promptMarker
      ? userText.reduce((count, text) => count + countOccurrences(text, promptMarker), 0)
      : 0,
  }
}

export function classifyOpenCodeSqliteTranscriptRows(rows, { finalMarker, promptMarker } = {}) {
  const messageRoles = new Map()
  for (const row of rows ?? []) {
    if (row?.kind !== "message" || typeof row.id !== "string") continue
    const message = sqliteRowData(row)
    const role = normalizedOpenCodeRole(message?.role)
    if (role) messageRoles.set(row.id, role)
  }

  const assistantText = []
  const assistantTools = []
  const assistantReasoning = []
  const userText = []
  for (const row of rows ?? []) {
    if (row?.kind !== "part") continue
    const part = sqliteRowData(row)
    const role = messageRoles.get(row.message_id)
    const partType = typeof part?.type === "string" ? part.type.trim().toLowerCase() : null
    if (role === "assistant" && partType === "text") {
      assistantText.push(openCodePartText(part))
    } else if (role === "assistant" && partType === "tool") {
      assistantTools.push(JSON.stringify(part))
    } else if (role === "assistant" && partType === "reasoning") {
      assistantReasoning.push(openCodePartText(part))
    } else if (role === "user" && partType === "text") {
      userText.push(openCodePartText(part))
    }
  }

  return classifiedTranscriptEvidence({
    assistantText,
    userText,
    toolEvents: assistantTools,
    reasoningText: assistantReasoning,
    finalMarker,
    promptMarker,
  })
}

function sqliteRowData(row) {
  if (row?.data && typeof row.data === "object") return row.data
  return typeof row?.data === "string" ? parseJson(row.data) : null
}

function normalizedOpenCodeRole(role) {
  if (typeof role !== "string") return null
  const normalized = role.trim().toLowerCase()
  return normalized === "assistant" || normalized === "user" ? normalized : null
}

function openCodePartText(part) {
  if (typeof part?.text === "string") return part.text
  if (typeof part?.content === "string") return part.content
  return ""
}

function markersInTexts(markers, texts) {
  return markers.filter((marker) => texts.some((text) => text.includes(marker)))
}

export function sqliteString(value) {
  return String(value).replace(/'/g, "''")
}

export function captureCommand(command, args, { maxBytes = 1024 * 1024 } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ["ignore", "pipe", "pipe"] })
    const stdout = []
    const stderr = []
    let stdoutBytes = 0
    let stderrBytes = 0
    child.stdout.on("data", (chunk) => {
      stdoutBytes += chunk.length
      if (stdoutBytes <= maxBytes) stdout.push(chunk)
      if (stdoutBytes > maxBytes) child.kill("SIGTERM")
    })
    child.stderr.on("data", (chunk) => {
      stderrBytes += chunk.length
      if (stderrBytes <= maxBytes) stderr.push(chunk)
    })
    child.on("error", reject)
    child.on("close", (code, signal) => {
      const result = {
        ok: code === 0,
        code,
        signal,
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8"),
      }
      if (result.ok) resolve(result)
      else reject(new Error(`${command} exited with ${signal ?? code}: ${result.stderr}`))
    })
  })
}

export async function findProviderTranscriptPath(provider, providerSessionId) {
  const roots = providerTranscriptRoots(provider)
  for (const root of roots) {
    const candidates = await providerTranscriptCandidates(provider, root)
    for (const candidate of candidates) {
      if (await providerTranscriptMatches(provider, candidate, providerSessionId)) {
        return candidate
      }
    }
  }
  return null
}

export function providerTranscriptRoots(provider) {
  const home = os.homedir()
  if (provider === "codex") {
    return [process.env.CODEX_HOME || path.join(home, ".codex")]
  }
  if (provider === "claude") {
    return [process.env.CLAUDE_HOME || path.join(home, ".claude")]
  }
  if (provider === "opencode") {
    return dedupe([
      process.env.OPENCODE_DATA_HOME,
      process.env.XDG_DATA_HOME ? path.join(process.env.XDG_DATA_HOME, "opencode") : null,
      path.join(home, ".local", "share", "opencode"),
      path.join(home, ".config", "opencode"),
    ].filter(Boolean))
  }
  return []
}

export async function providerTranscriptCandidates(provider, root) {
  const specs = {
    codex: [
      { root: path.join(root, "archived_sessions"), depth: 4, extensions: new Set([".jsonl"]) },
      { root: path.join(root, "sessions"), depth: 4, extensions: new Set([".jsonl"]) },
    ],
    claude: [
      { root: path.join(root, "projects"), depth: 3, extensions: new Set([".jsonl"]) },
    ],
    opencode: [
      { root, depth: 5, extensions: new Set([".json", ".jsonl"]) },
    ],
  }[provider] ?? []
  const files = []
  for (const spec of specs) {
    files.push(...await fileCandidates(spec.root, spec.depth, spec.extensions, provider === "opencode"))
  }
  await primeStatCache(files)
  return sortRecentFiles(dedupe(files)).slice(0, 1_000)
}

export async function providerTranscriptMatches(provider, file, providerSessionId) {
  if (path.basename(file) === `${providerSessionId}.json` || path.basename(file) === `${providerSessionId}.jsonl`) {
    return true
  }
  if (path.basename(file).includes(providerSessionId)) {
    return true
  }
  const text = await readFile(file, "utf8").catch(() => "")
  if (!text) return false
  if (provider === "codex") {
    return jsonlValues(text).some((value) => value?.type === "session_meta"
      && stringField(value.payload, ["id", "session_id", "sessionId"]) === providerSessionId)
  }
  if (provider === "claude") {
    return jsonlValues(text).some((value) => stringField(value, ["sessionId", "session_id"]) === providerSessionId)
  }
  if (provider === "opencode") {
    if (path.extname(file) === ".jsonl") {
      return jsonlValues(text).some((value) => stringField(value, ["sessionID", "sessionId", "session_id", "id"]) === providerSessionId)
    }
    const value = parseJson(text)
    if (!value) return false
    return stringField(value, ["id", "sessionID", "sessionId", "session_id"]) === providerSessionId
  }
  return false
}

export async function fileCandidates(root, depth, extensions, opencodeNamesOnly = false) {
  if (depth <= 0) return []
  const entries = await readdir(root, { withFileTypes: true }).catch(() => [])
  const files = []
  for (const entry of entries) {
    if (entry.name === "node_modules") continue
    const file = path.join(root, entry.name)
    if (entry.isDirectory()) {
      files.push(...await fileCandidates(file, depth - 1, extensions, opencodeNamesOnly))
    } else if (entry.isFile() && extensions.has(path.extname(entry.name))) {
      const lower = file.toLowerCase()
      if (!opencodeNamesOnly || lower.includes("session") || lower.includes("conversation") || lower.includes("message") || lower.endsWith(".jsonl")) {
        files.push(file)
      }
    }
  }
  return files
}

export function sortRecentFiles(files) {
  return files.sort((left, right) => fileModifiedMs(right) - fileModifiedMs(left) || left.localeCompare(right))
}

export function fileModifiedMs(file) {
  return statSyncCache.get(file) ?? 0
}

export async function primeStatCache(files) {
  await Promise.all(files.map(async (file) => {
    const metadata = await stat(file).catch(() => null)
    statSyncCache.set(file, metadata?.mtimeMs ?? 0)
  }))
}

export function jsonlValues(text) {
  return String(text)
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      try {
        return JSON.parse(line)
      } catch {
        return null
      }
    })
    .filter(Boolean)
}

export function parseJson(text) {
  try {
    return JSON.parse(text)
  } catch {
    return null
  }
}

export function stringField(value, keys) {
  if (!value || typeof value !== "object") return null
  for (const key of keys) {
    const field = value[key]
    if (typeof field === "string" && field.length > 0) return field
  }
  return null
}

export function pathJoin(...parts) {
  return path.join(...parts)
}

export function pathExt(file) {
  const extension = path.extname(file)
  return extension || ".txt"
}

export function assertProviderTranscript(result, transcript, label) {
  result.assertions.push(assertion(`${label} file found`, Boolean(transcript?.found), transcript?.reason ?? transcript?.path))
  if (!transcript?.found) return
  result.assertions.push(assertion(`${label} saw all assistant markers`, transcript.assistantMarkersSeen.length === 20, transcript.assistantMarkersSeen))
  result.assertions.push(assertion(`${label} saw all tool markers`, transcript.toolMarkersSeen.length === 20, transcript.toolMarkersSeen))
  result.assertions.push(assertion(`${label} saw final summary marker`, transcript.finalSeen, transcript.finalSeen))
  result.assertions.push(assertion(`${label} saw external prompt marker`, transcript.promptOccurrences >= 1, transcript.promptOccurrences))
}

export function assertSurface(result, surfaceResult, label) {
  if (!surfaceResult) {
    result.assertions.push(fail(`${label} monitor ran`, "missing monitor result"))
    return
  }
  result.assertions.push(assertion(`${label} saw all assistant markers`, surfaceResult.assistantMarkersSeen.length === 20, surfaceResult.assistantMarkersSeen))
  result.assertions.push(assertion(`${label} saw all tool markers`, surfaceResult.toolMarkersSeen.length === 20, surfaceResult.toolMarkersSeen))
  result.assertions.push(assertion(`${label} saw final summary marker`, surfaceResult.finalSeen, surfaceResult.finalSeen))
  result.assertions.push(assertion(`${label} rendered external prompt marker exactly once`, surfaceResult.promptOccurrenceMax === 1, surfaceResult.promptOccurrenceMax))
  if (label.includes("web")) {
    result.assertions.push(assertion(`${label} stayed near bottom while tailing`, surfaceResult.preFinalMaxBottomDistance < 260, surfaceResult.preFinalMaxBottomDistance))
  }
}

export function assertLiveObservation(result, surfaceResult, label, options = {}) {
  if (!surfaceResult) return
  result.assertions.push(assertion(`${label} sampled turn before final summary`, surfaceResult.preFinalSampleCount > 0, {
    preFinalSampleCount: surfaceResult.preFinalSampleCount,
    firstFinalSampleIndex: surfaceResult.firstFinalSampleIndex,
  }))
  const preFinalStatuses = surfaceResult.preFinalStatuses.map(normalizeLifecycleStatus)
  result.assertions.push(assertion(`${label} observed active pre-final lifecycle`, preFinalStatuses.includes("WORKING"), preFinalStatuses))
  if (options.requireContent === false) return
  const sawPreFinalContent = surfaceResult.preFinalMaxAssistantMarkers > 0 || surfaceResult.preFinalMaxToolMarkers > 0
  result.assertions.push(assertion(`${label} observed live pre-final content`, sawPreFinalContent, {
    assistantMarkers: surfaceResult.preFinalMaxAssistantMarkers,
    toolMarkers: surfaceResult.preFinalMaxToolMarkers,
  }))
}

export function assertBadgeLifecycle(result, surfaceResult, label) {
  if (!surfaceResult) return
  const statuses = surfaceResult.statuses.map(normalizeLifecycleStatus)
  result.assertions.push(assertion(`${label} observed WORKING`, statuses.includes("WORKING"), statuses))
  result.assertions.push(assertion(`${label} ended IDLE or unknown-idle-compatible`, statuses.at(-1) === "IDLE" || statuses.at(-1) === "IDLE/DONE" || statuses.at(-1) === "DONE", statuses.at(-1)))
  const firstWorking = statuses.indexOf("WORKING")
  const finalIndex = surfaceResult.samples.findIndex((sample) => sample.finalSeen)
  const prematureIdle = firstWorking >= 0 && finalIndex > firstWorking
    ? statuses.slice(firstWorking, finalIndex).some((status) => status === "IDLE" || status === "DONE")
    : false
  result.assertions.push(assertion(`${label} did not go idle before final summary`, !prematureIdle, statuses))
}

export function assertWebTurnCollapse(result, webResult) {
  result.assertions.push(assertion("web collapsed completed turn after final summary", webResult.finalMaxTurnCollapsedCount > 0, {
    finalCollapsed: webResult.finalMaxTurnCollapsedCount,
    finalExpanded: webResult.finalMaxTurnExpandedCount,
    finalBlobCollapsed: webResult.finalMaxBlobCollapsedCount,
  }))
}

export function providerLimitations(provider, monitorResults, context = {}) {
  const providerAssistantOutputMissing = semanticProviderTranscriptMisses(
    monitorResults.providerTranscript,
    "assistantMarkersSeen",
    requiredAssistantMarkers.length,
  )
  const providerToolOutputMissing = semanticProviderTranscriptMisses(
    monitorResults.providerTranscript,
    "toolMarkersSeen",
    requiredToolMarkers.length,
  )
  const surfaceTexts = new Map([
    ["provider_transcript", monitorResults.providerTranscript?.found ? readArtifactTextSync(monitorResults.providerTranscript) : ""],
    ["kernel", monitorResults.kernel?.samples?.map((sample) => sample.text ?? "").join("\n") ?? ""],
    ["web", monitorResults.web?.samples?.map((sample) => sample.text ?? "").join("\n") ?? ""],
    ["tui", monitorResults.tui?.samples?.map((sample) => sample.text ?? "").join("\n") ?? ""],
  ])
  const combinedText = [...surfaceTexts.values()].join("\n").toLowerCase()
  const surfacesWithText = (predicate) => [...surfaceTexts.entries()]
    .filter(([, text]) => text && predicate(text.toLowerCase()))
    .map(([surface]) => surface)
  const metadataReport = [
    metadataAvailability({
      provider,
      context,
      metadata: "status/running_state",
      observed: Boolean(monitorResults.kernel?.statuses?.length || monitorResults.web?.statuses?.length || monitorResults.tui?.statuses?.length),
      surfaces: ["kernel", monitorResults.web ? "web" : null, monitorResults.tui ? "tui" : null].filter(Boolean),
      observedNote: "Chariox observed external turn lifecycle from WORKING to final IDLE/DONE.",
      missingNote: "No lifecycle statuses were sampled during the drill.",
      missingClassification: "chariox_bug",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "assistant_text",
      observed: (monitorResults.kernel?.assistantMarkersSeen?.length ?? 0) === 20,
      surfaces: [
        (monitorResults.providerTranscript?.assistantMarkersSeen?.length ?? 0) === 20 ? "provider_transcript" : null,
        (monitorResults.kernel?.assistantMarkersSeen?.length ?? 0) === 20 ? "kernel" : null,
        (monitorResults.web?.assistantMarkersSeen?.length ?? 0) === 20 ? "web" : null,
        (monitorResults.tui?.assistantMarkersSeen?.length ?? 0) === 20 ? "tui" : null,
      ].filter(Boolean),
      observedNote: "All assistant progress markers were visible in imported external history.",
      missingNote: providerAssistantOutputMissing
        ? "The provider transcript did not contain all required assistant text markers, so Chariox had no provider output to import."
        : "Assistant text did not fully appear in imported external history.",
      missingClassification: providerAssistantOutputMissing ? "provider_output_limitation" : "chariox_bug",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "tool_calls",
      observed: (monitorResults.kernel?.toolMarkersSeen?.length ?? 0) === 20,
      surfaces: [
        (monitorResults.providerTranscript?.toolMarkersSeen?.length ?? 0) === 20 ? "provider_transcript" : null,
        (monitorResults.kernel?.toolMarkersSeen?.length ?? 0) === 20 ? "kernel" : null,
        (monitorResults.web?.toolMarkersSeen?.length ?? 0) === 20 ? "web" : null,
        (monitorResults.tui?.toolMarkersSeen?.length ?? 0) === 20 ? "tui" : null,
      ].filter(Boolean),
      observedNote: "All marked provider tool calls were visible in imported external history.",
      missingNote: providerToolOutputMissing
        ? "The provider transcript did not contain all required tool markers, so Chariox had no provider tool events to import."
        : "Tool-call markers did not fully appear in imported external history.",
      missingClassification: providerToolOutputMissing ? "provider_output_limitation" : "chariox_bug",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "tool_results",
      observed: requiredToolMarkers.some((marker) => combinedText.includes(marker.toLowerCase())),
      surfaces: surfacesWithText((text) => requiredToolMarkers.some((marker) => text.includes(marker.toLowerCase()))),
      observedNote: "Provider tool result/output text was visible at least through marked tool-call output.",
      missingNote: "Tool result/output text was not observed in imported external history.",
      missingClassification: "provider_persistence_or_adapter_limitation",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "reasoning/thinking_summaries",
      observed: /\b(reasoning|thinking|thought|summary unavailable|visible summary)\b/i.test(combinedText),
      surfaces: surfacesWithText((text) => /\b(reasoning|thinking|thought|summary unavailable|visible summary)\b/i.test(text)),
      observedNote: "Reasoning/thinking metadata or an explicit unavailable-summary entry was visible.",
      missingNote: "Reasoning/thinking summaries were not observed in imported external history.",
      missingClassification: "provider_persistence_limitation",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "timestamps",
      observed: Boolean(monitorResults.kernel?.samples?.some((sample) => sample.at) || monitorResults.providerTranscript?.found),
      surfaces: ["provider_transcript", "kernel"].filter((surface) => surface !== "provider_transcript" || monitorResults.providerTranscript?.found),
      observedNote: "Observation timestamps and/or provider transcript timestamps were captured.",
      missingNote: "No timestamps were captured for provider or Chariox observations.",
      missingClassification: "drill_observation_limitation",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "token_usage",
      observed: /\b(token|usage|input_tokens|output_tokens|cached_input_tokens)\b/i.test(combinedText),
      surfaces: surfacesWithText((text) => /\b(token|usage|input_tokens|output_tokens|cached_input_tokens)\b/i.test(text)),
      observedNote: "Token usage metadata was visible in provider/imported history.",
      missingNote: "Token usage metadata was not observed in imported external history.",
      missingClassification: "provider_persistence_limitation",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "model_identity",
      observed: Boolean(context.model) || /\b(model|gpt|sonnet|kimi|claude)\b/i.test(combinedText),
      surfaces: context.model ? ["drill_config"] : surfacesWithText((text) => /\b(model|gpt|sonnet|kimi|claude)\b/i.test(text)),
      observedNote: `Model identity was available to the drill as ${context.model ?? "provider transcript metadata"}.`,
      missingNote: "Model identity was not observed.",
      missingClassification: "drill_observation_limitation",
    }),
    metadataAvailability({
      provider,
      context,
      metadata: "final_completion_or_error_state",
      observed: Boolean(monitorResults.kernel?.finalSeen || context.providerExit),
      surfaces: ["provider_process", "kernel"].filter((surface) => surface !== "kernel" || monitorResults.kernel?.finalSeen),
      observedNote: `Provider exit and final marker were captured${context.providerExit ? ` with exit ${JSON.stringify(context.providerExit)}` : ""}.`,
      missingNote: "Final completion/error state was not captured.",
      missingClassification: "chariox_bug",
    }),
  ]
  if (!monitorResults.web) metadataReport.push({ provider, surface: "web", status: "skipped", classification: "drill_observation_limitation" })
  if (!monitorResults.tui) metadataReport.push({ provider, surface: "tui", status: "skipped", classification: "drill_observation_limitation" })
  return metadataReport
}

function semanticProviderTranscriptMisses(transcript, field, expectedCount) {
  return transcript?.found === true
    && transcript.semanticEvidence === true
    && (transcript[field]?.length ?? 0) < expectedCount
}

export function metadataAvailability({ provider, context, metadata, observed, surfaces, observedNote, missingNote, missingClassification }) {
  return {
    provider,
    providerSessionId: context.providerSessionId ?? null,
    externalSessionId: context.externalSessionId ?? null,
    charioxSessionId: context.charioxSessionId ?? null,
    agentId: context.agentId ?? null,
    metadata,
    status: observed ? "observed" : "not_observed",
    classification: observed ? "available_to_chariox" : missingClassification,
    surfaces: dedupe((surfaces ?? []).filter(Boolean)),
    note: observed ? observedNote : missingNote,
  }
}

export function readArtifactTextSync(transcript) {
  const artifactPath = transcript.artifactPath ?? transcript.path
  if (!artifactPath) return ""
  try {
    return readFileSync(artifactPath, "utf8")
  } catch {
    return [
      transcript.path ?? "",
      transcript.artifactPath ?? "",
      ...(transcript.assistantMarkersSeen ?? []),
      ...(transcript.toolMarkersSeen ?? []),
      transcript.finalSeen ? "final" : "",
    ].join("\n")
  }
}

export async function writeFinalReport(root, summary) {
  const report = [
    "# External Provider Live Parity Drill Report",
    "",
    "## Overall Result",
    "",
    `- Result: ${summary.ok ? "PASS" : "FAIL"}`,
    `- Created: ${summary.createdAt}`,
    `- Artifact root: ${summary.artifactRoot}`,
    `- Kernel URL: ${summary.kernelUrl}`,
    `- Web URL: ${summary.webUrl}`,
    `- Workspace: ${summary.workspace}`,
    "",
    "## Provider Matrix",
    "",
    "| Provider | Model | Provider session | Chariox session | Agent | Result | Failed assertions |",
    "| --- | --- | --- | --- | --- | --- | --- |",
    ...summary.results.map((result) => {
      const failed = (result.assertions ?? []).filter((assertion) => !assertion.passed)
      return [
        result.provider,
        result.model,
        result.providerSessionId ?? result.externalSessionId ?? "",
        result.charioxSessionId ?? "",
        result.agentId ?? "",
        result.ok ? "PASS" : "FAIL",
        failed.map((assertion) => assertion.name).join("; "),
      ].map(markdownCell).join(" | ").replace(/^/, "| ").replace(/$/, " |")
    }),
    "",
    "## Web Terminal Evidence",
    "",
    ...surfaceEvidence(summary.results, "web"),
    "",
    "## TUI Evidence",
    "",
    ...surfaceEvidence(summary.results, "tui"),
    "",
    "## Artifact Paths",
    "",
    ...summary.results.flatMap((result) => [
      `- ${result.provider}: ${result.artifactDir}`,
      `  - Prompt: ${path.join(result.artifactDir, "prompt.txt")}`,
      `  - Provider stdout: ${path.join(result.artifactDir, "provider.stdout.log")}`,
      `  - Provider stderr: ${path.join(result.artifactDir, "provider.stderr.log")}`,
      `  - Web final screenshot: ${path.join(result.artifactDir, "web-final.png")}`,
      `  - Provider transcript artifact: ${result.evidence?.providerTranscript?.artifactPath ?? "not captured"}`,
    ]),
    "",
    "## Provider Limitations And Clarifications",
    "",
    "This section is intentionally last. It distinguishes Chariox bugs from provider-native metadata limits and drill-observation limits.",
    "",
    "| Provider | Metadata | Status | Classification | Surfaces | Clarification | IDs |",
    "| --- | --- | --- | --- | --- | --- | --- |",
    ...summary.providerLimitations.map((entry) => [
      entry.provider ?? "",
      entry.metadata ?? entry.surface ?? "",
      entry.status ?? "",
      entry.classification ?? "",
      (entry.surfaces ?? [entry.surface]).filter(Boolean).join(", "),
      entry.note ?? "",
      [
        entry.providerSessionId ? `provider=${entry.providerSessionId}` : null,
        entry.externalSessionId ? `external=${entry.externalSessionId}` : null,
        entry.charioxSessionId ? `chariox=${entry.charioxSessionId}` : null,
        entry.agentId ? `agent=${entry.agentId}` : null,
      ].filter(Boolean).join("; "),
    ].map(markdownCell).join(" | ").replace(/^/, "| ").replace(/$/, " |")),
    "",
  ].join("\n")
  await writeFile(path.join(root, "final-report.md"), report, "utf8")
}

export function surfaceEvidence(results, surface) {
  const lines = []
  for (const result of results) {
    const monitor = result.evidence?.monitors?.find((entry) => entry.surface === surface)
    if (!monitor) {
      lines.push(`- ${result.provider}: ${surface} monitor was skipped or unavailable.`)
      continue
    }
    const statuses = monitor.statuses ?? []
    lines.push([
      `- ${result.provider}:`,
      `result=${result.ok ? "PASS" : "FAIL"}`,
      `samples=${monitor.sampleCount}`,
      `assistant=${monitor.assistantMarkersSeen?.length ?? 0}/20`,
      `tools=${monitor.toolMarkersSeen?.length ?? 0}/20`,
      `final=${monitor.finalSeen ? "yes" : "no"}`,
      `first_status=${statuses[0] ?? "unknown"}`,
      `last_status=${statuses.at(-1) ?? "unknown"}`,
      `prompt_occurrence_max=${monitor.promptOccurrenceMax ?? "unknown"}`,
      surface === "web" ? `pre_final_max_bottom_distance=${monitor.preFinalMaxBottomDistance ?? "unknown"}` : null,
      surface === "web" ? `max_bottom_distance=${monitor.maxBottomDistance ?? "unknown"}` : null,
      `pre_final_samples=${monitor.preFinalSampleCount ?? 0}`,
    ].filter(Boolean).join(" "))
  }
  return lines
}

const statSyncCache = new Map()
